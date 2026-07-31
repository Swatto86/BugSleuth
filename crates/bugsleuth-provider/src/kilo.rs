//! Kilo CLI adapter.
//!
//! The third vendor, and the awkward one. Two things it does not have that the
//! others do, both of which change the design rather than just the argv:
//!
//! **No output-schema flag.** Claude takes a JSON Schema inline and Codex takes
//! one as a file; Kilo takes neither. The schema therefore has to be described
//! in the prompt and the reply validated afterwards, which is strictly weaker —
//! expect a higher rate of malformed replies from this vendor than the others.
//!
//! **No read-only mode.** Codex has `--sandbox read-only` and Claude has a tool
//! allowlist. Kilo's permissions come from the *user's own global config*, and
//! on the machine this was written against both candidate agents (`ask` and
//! `plan`) were configured to allow everything. There is no per-invocation flag
//! that overrides it.
//!
//! So a Kilo sweep is never pointed at the repository under review. It is given
//! a throwaway git worktree, which the caller deletes afterwards. That is
//! enforced here rather than left to the caller: [`KiloSweep`] takes a
//! `worktree`, not a `repo`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::RawFindings;
use serde_json::Value;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

mod discover;

pub(crate) const VENDOR: &str = "kilo";

pub struct KiloSweep<'a> {
    /// A throwaway checkout the model may safely write to. **Never** the
    /// repository under review — see the module note above.
    pub worktree: &'a Path,
    /// Model in Kilo's `provider/model` form. Empty means its configured default.
    pub model: &'a str,
    /// The brief. Must already describe the required JSON shape, because this
    /// CLI cannot be given a schema to enforce.
    pub brief: &'a str,
    pub timeout: Duration,
    pub binary: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct KiloResult {
    pub findings: RawFindings,
}

pub async fn sweep(spec: KiloSweep<'_>) -> Result<KiloResult, ProviderError> {
    let binary = match spec.binary {
        Some(path) => PathBuf::from(path),
        None => discover::resolve_binary().ok_or_else(not_found)?,
    };

    let args = build_args(&spec);
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &args,
        cwd: spec.worktree,
        stdin: Some(spec.brief.as_bytes()),
        env: &[],
        timeout: spec.timeout,
        what: "kilo CLI",
    })
    .await?;

    if !output.succeeded() {
        let code = output.code.unwrap_or(-1);
        let message = preview(output.stderr.trim(), 2000);
        return Err(if message.is_empty() {
            ProviderError::FailedSilently {
                vendor: VENDOR,
                code,
            }
        } else {
            ProviderError::Failed {
                vendor: VENDOR,
                code,
                message,
            }
        });
    }

    let text = assistant_text(&output.stdout);
    if text.trim().is_empty() {
        return Err(ProviderError::Empty(VENDOR));
    }
    Ok(KiloResult {
        findings: crate::json::structured(&Value::String(text))?,
    })
}

/// Build the non-interactive argv.
///
/// `--pure` is Kilo's nearest equivalent to the other vendors' safe modes: it
/// skips external plugins, so a plugin installed on this machine cannot change
/// what the review does. It does **not** neutralise agent permissions, which is
/// why the worktree exists.
fn build_args(spec: &KiloSweep<'_>) -> Vec<String> {
    let mut args: Vec<String> = [
        "run", "--auto", "--pure", "--format", "json", "--agent", "ask",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    // Pin the working directory explicitly as well as via the spawned process's
    // cwd. Kilo resolves some paths from `--dir` rather than the process cwd,
    // and a mismatch between the two would have it review a different tree than
    // the one whose anchors we later verify against.
    args.push("--dir".into());
    args.push(spec.worktree.to_string_lossy().into_owned());

    let model = spec.model.trim();
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.to_string());
    }
    args
}

/// The assistant's final message, from Kilo's NDJSON event stream.
///
/// **Not a concatenation.** Each `text` event carries the *complete* text of its
/// message, not an incremental delta, and Kilo emits the same message more than
/// once. Appending them produced `{"findings":[]}{"findings":[]}` — two valid
/// objects glued into one invalid document — and the first real Kilo sweep
/// failed on exactly that.
///
/// So events are grouped by message id, the latest text wins within a message,
/// and the last message is the answer. Earlier messages are the agent narrating
/// its progress ("I'll start by exploring the repository structure"), which is
/// not the reply and must not be mixed into it.
///
/// The payload is nested under a `part` object in current versions, with an
/// older flat shape still in the wild. Both are read, and unparseable lines are
/// skipped rather than failing the sweep: the wire format is still moving
/// upstream and one unknown event should not discard a completed review.
fn assistant_text(stdout: &str) -> String {
    // (message id, latest text) in first-seen order. A handful of messages, so
    // a linear scan is cheaper than a map and keeps the ordering for free.
    let mut messages: Vec<(String, String)> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event["type"].as_str() != Some("text") {
            continue;
        }
        let Some(text) = event["part"]["text"]
            .as_str()
            .or_else(|| event["text"].as_str())
        else {
            continue;
        };
        // Messages without an id are treated as one anonymous message rather
        // than being dropped; older event shapes carry no id at all.
        let id = event["part"]["messageID"]
            .as_str()
            .or_else(|| event["messageID"].as_str())
            .unwrap_or("")
            .to_string();

        match messages.iter_mut().find(|(seen, _)| *seen == id) {
            Some((_, existing)) => *existing = text.to_string(),
            None => messages.push((id, text.to_string())),
        }
    }

    messages.pop().map(|(_, text)| text).unwrap_or_default()
}

fn not_found() -> ProviderError {
    ProviderError::NotFound {
        vendor: VENDOR,
        hint: "Install it with `npm install -g @kilocode/cli` and sign in with `kilo auth`, or \
               pass an explicit binary path."
            .to_string(),
    }
}

/// Check the CLI exists and can run. Free — starts no model.
pub async fn probe() -> Result<String, ProviderError> {
    let binary = discover::resolve_binary().ok_or_else(not_found)?;
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &["--version".to_string()],
        cwd: Path::new("."),
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(60),
        what: "kilo CLI",
    })
    .await?;

    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 500),
        });
    }
    // Kilo prints a banner before the version; keep the last non-empty line.
    let version = output
        .stdout
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("unknown")
        .trim()
        .to_string();
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec<'a>(model: &'a str) -> KiloSweep<'a> {
        KiloSweep {
            worktree: Path::new("/tmp/wt"),
            model,
            brief: "",
            timeout: Duration::from_secs(60),
            binary: None,
        }
    }

    #[test]
    fn external_plugins_are_skipped_so_the_machine_cannot_change_the_review() {
        assert!(build_args(&spec("")).iter().any(|a| a == "--pure"));
    }

    #[test]
    fn the_working_directory_is_pinned_explicitly() {
        let args = build_args(&spec(""));
        let dir = args
            .iter()
            .position(|a| a == "--dir")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        assert_eq!(dir, Some("/tmp/wt"));
    }

    #[test]
    fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
        assert!(!build_args(&spec("   ")).iter().any(|a| a == "-m"));
    }

    #[test]
    fn a_message_repeated_by_the_stream_is_not_glued_to_itself() {
        // Kilo really does emit the same complete message twice. Concatenating
        // these produced `{"findings":[]}{"findings":[]}` and the first real
        // Kilo sweep failed to parse.
        let stdout = concat!(
            r#"{"type":"step_start"}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m1","text":"{\"findings\":[]}"}}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m1","text":"{\"findings\":[]}"}}"#,
            "\n",
            r#"{"type":"step_finish"}"#,
        );
        assert_eq!(assistant_text(stdout), r#"{"findings":[]}"#);
    }

    #[test]
    fn narration_from_earlier_messages_is_not_mixed_into_the_answer() {
        let stdout = concat!(
            r#"{"type":"text","part":{"messageID":"m1","text":"I'll start by exploring."}}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m2","text":"{\"findings\":[]}"}}"#,
        );
        assert_eq!(assistant_text(stdout), r#"{"findings":[]}"#);
    }

    #[test]
    fn the_older_flat_event_shape_is_still_read() {
        let stdout = r#"{"type":"text","text":"hello"}"#;
        assert_eq!(assistant_text(stdout), "hello");
    }

    #[test]
    fn a_stream_with_no_text_events_yields_nothing_rather_than_panicking() {
        assert_eq!(assistant_text(r#"{"type":"step_finish"}"#), "");
        assert_eq!(assistant_text(""), "");
    }

    #[test]
    fn a_malformed_event_is_skipped_rather_than_discarding_the_whole_review() {
        let stdout = concat!(
            "not json at all\n",
            r#"{"type":"text","part":{"text":"kept"}}"#,
            "\n{ broken json\n",
        );
        assert_eq!(assistant_text(stdout), "kept");
    }
}
