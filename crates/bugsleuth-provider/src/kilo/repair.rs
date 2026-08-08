//! Asking Kilo to reshape a reply that did not match the schema.
//!
//! Kilo is the one vendor whose output cannot be constrained — it is handed the
//! schema as prose and asked nicely. When it answers in the wrong shape the
//! whole sweep is discarded, and that sweep already did the expensive part:
//! it read the repository and found the defects. Three sweeps were lost that
//! way in one day, to `category` instead of `title`, to no JSON object at all,
//! and to a wrong field type.
//!
//! So a malformed reply gets one chance to be reshaped. The repair invocation
//! is deliberately cheap and deliberately narrow: no repository access, no
//! reviewing, just "here is what you wrote and here is the shape it must take".
//!
//! **It must not be allowed to invent.** A model asked to fix a document it
//! cannot fully parse will happily fill the gaps, and an invented finding is
//! precisely what this project exists to prevent. The instruction says so
//! bluntly, and the anchor check still runs afterwards — a fabricated snippet
//! is discarded there regardless of how it came to exist.

use std::time::Duration;

use bugsleuth_domain::{RawFindings, finding_schema};

use crate::error::ProviderError;
use crate::process::{self, Invocation};

use super::{discover, events};

/// Removes the repair's scratch directory however the function leaves.
pub(super) struct Cleanup(pub(super) std::path::PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An empty directory nobody else is using, for a process that must reach
/// nothing. `create_dir` rather than `create_dir_all`: failing when the path
/// exists is what proves it is ours.
pub(super) fn empty_dir() -> Option<std::path::PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);

    let base = std::env::temp_dir();
    let pid = std::process::id();
    for attempt in 0..64 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("bugsleuth-kilo-repair-{pid}-{n}-{attempt}"));
        if std::fs::create_dir(&dir).is_ok() {
            return Some(dir);
        }
    }
    None
}

/// One retry only. A model that cannot produce the shape twice will not produce
/// it on a third attempt, and each try costs real time.
const ATTEMPTS: usize = 1;

/// Try to reshape a malformed reply into the required structure.
///
/// Returns `None` when the repair itself fails, so the caller reports the
/// *original* schema error rather than a confusing second one — the first
/// message is the one that says what the model actually got wrong.
pub(super) async fn reshape(
    malformed: &str,
    binary: &std::path::Path,
    model: &str,
    timeout: Duration,
) -> Option<RawFindings> {
    // Empty, exclusively created, and removed when the repair is done.
    let sandbox = empty_dir()?;
    let _guard = Cleanup(sandbox.clone());

    // Oversized replies cannot be repaired from a truncated prefix without
    // silently dropping whatever came after the cut, so those are not attempted
    // at all — the caller reports the original schema error instead.
    let brief = prompt(malformed)?;
    for _ in 0..ATTEMPTS {
        let mut args: Vec<String> = [
            "run", "--auto", "--pure", "--format", "json", "--agent", "ask",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        // **Not the worktree.** This module's own description says the repair
        // gets no repository access, and for a while it handed Kilo the tree
        // under review anyway — with `--auto`, so any tool call it made was
        // approved without asking. The repair reshapes text it was given; it
        // has no use for the repository, and Kilo cannot be told to keep its
        // hands off one, so it is pointed at an empty private directory where
        // there is nothing to reach.
        args.push("--dir".into());
        args.push(sandbox.to_string_lossy().into_owned());
        if !model.trim().is_empty() {
            args.push("-m".into());
            args.push(model.trim().to_string());
        }

        let output = process::run(Invocation {
            binary: &binary.to_string_lossy(),
            args: &args,
            cwd: &sandbox,
            stdin: Some(brief.as_bytes()),
            env: &[],
            // Far shorter than a sweep: this reads nothing and writes one JSON
            // object. A repair that takes as long as a review is not a repair.
            timeout: timeout.min(Duration::from_secs(300)),
            what: "kilo CLI (repair)",
        })
        .await
        .ok()?;

        if !output.succeeded() {
            continue;
        }
        let text = events::assistant_text(&output.stdout);
        if let Ok(findings) =
            crate::json::structured::<RawFindings>(&serde_json::Value::String(text))
        {
            return Some(findings);
        }
    }
    None
}

/// The largest malformed reply a repair will accept whole. Beyond this the reply
/// cannot be handed over without truncation, and a repair of a truncated prefix
/// silently loses every finding past the cut — so oversized replies are refused
/// rather than partially repaired.
const MAX_REPAIR_CHARS: usize = 20_000;

/// The repair prompt, or `None` when the reply is too large to repair whole.
fn prompt(malformed: &str) -> Option<String> {
    if malformed.chars().count() > MAX_REPAIR_CHARS {
        return None;
    }
    Some(format!(
        "Your previous reply was rejected because it did not match the required \
         JSON structure. Reshape it. Do NOT review any code again, do NOT open \
         any files, and do NOT add, remove or embellish any finding.\n\n\
         **Invent nothing.** If a required field is genuinely missing from what \
         you wrote, drop that finding entirely rather than guessing at it. A \
         fabricated finding is far worse than a lost one, and every snippet is \
         checked against the real file afterwards regardless.\n\n\
         Reply with a single JSON object and nothing else — no prose, no code \
         fence. It must validate against this schema:\n\n{}\n\n\
         Here is what you wrote:\n\n{}\n",
        serde_json::to_string_pretty(&finding_schema()).unwrap_or_default(),
        malformed,
    ))
}

/// Locate the CLI for a repair attempt.
pub(super) fn binary(explicit: Option<&str>) -> Option<std::path::PathBuf> {
    match explicit {
        Some(path) => Some(std::path::PathBuf::from(path)),
        None => discover::resolve_binary(),
    }
}

/// The error a caller should report when a repair did not help.
pub(super) fn original(error: ProviderError) -> ProviderError {
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_at_the_limit_is_repaired_whole_and_one_over_is_refused() {
        // The truncation this replaces: an oversized reply was handed over as a
        // 20k-char preview, so any finding past the cut was silently dropped and
        // the repair looked complete. At the limit the whole reply must be in the
        // prompt; one character over must refuse rather than repair a prefix.
        let at_limit = "x".repeat(MAX_REPAIR_CHARS);
        let brief = prompt(&at_limit).expect("a reply at the limit is repairable");
        assert!(
            brief.contains(&at_limit),
            "the reply at the limit was not included whole"
        );

        let over_limit = "x".repeat(MAX_REPAIR_CHARS + 1);
        assert!(
            prompt(&over_limit).is_none(),
            "an oversized reply was repaired from a truncated prefix"
        );
    }

    #[test]
    fn the_repair_prompt_forbids_inventing_and_forbids_reviewing_again() {
        // Both matter. Re-reviewing turns a cheap reshape into a second sweep,
        // and inventing produces exactly the confident fiction this tool exists
        // to keep out of a report.
        let text = prompt(r#"{"findings":[{"category":"correctness"}]}"#)
            .expect("a short reply is repairable");
        assert!(text.contains("Do NOT review any code again"));
        assert!(text.contains("Invent nothing"));
        assert!(text.contains("drop that finding entirely"));
        // It must carry the malformed reply, or there is nothing to reshape.
        assert!(text.contains("\"category\""));
        // And the schema, or the model is guessing at the target.
        assert!(text.contains("failure_scenario"));
    }

    #[test]
    fn the_repair_is_pointed_at_an_empty_directory_of_its_own() {
        // It reshapes text it was handed and has no use for the repository.
        // Kilo runs with --auto and cannot be restricted per invocation, so the
        // only lever is what is within reach: nothing.
        let a = empty_dir().expect("a scratch directory");
        let b = empty_dir().expect("a second scratch directory");
        assert_ne!(a, b, "two repairs shared a directory");
        assert!(a.is_dir() && b.is_dir());
        assert_eq!(
            std::fs::read_dir(&a).map(|d| d.count()).unwrap_or(1),
            0,
            "the repair directory was not empty"
        );
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn the_scratch_directory_is_removed_when_the_repair_ends() {
        let path = {
            let dir = empty_dir().expect("a scratch directory");
            let _guard = Cleanup(dir.clone());
            assert!(dir.is_dir());
            dir
        };
        assert!(!path.exists(), "the repair left its directory behind");
    }
}
