//! Codex's working files, and reading its failure events.
//!
//! Codex writes its final answer to a file rather than only to its event
//! stream, so unlike the other adapters it needs somewhere to put one. That
//! somewhere is the system temp area, never inside the repository being
//! changed: an apply must leave only the fix behind.

use std::path::{Path, PathBuf};

use crate::error::ProviderError;
use crate::process::{self, preview};

use super::VENDOR;

/// First error carried by a Codex event, if any.
pub(crate) fn event_error(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let event: serde_json::Value = serde_json::from_str(line).ok()?;
        match event["type"].as_str()? {
            "turn.failed" => event["error"]["message"].as_str().map(|s| preview(s, 2000)),
            "error" => event["message"].as_str().map(|s| preview(s, 2000)),
            _ => None,
        }
    })
}

/// The model's own account, or the CLI's own account of why there isn't one.
///
/// Shared by the read-only sweep and the write-capable apply: both read the
/// final message from a file Codex writes, and both prefer a structured event
/// on stdout over an empty stderr when that file is missing.
pub(crate) fn finish(
    output: Result<crate::process::CliOutput, crate::process::ProcessError>,
    answer_path: &Path,
) -> Result<String, ProviderError> {
    let output = match output {
        Ok(output) => output,
        Err(process::ProcessError::OutputTruncated { output, .. }) if output.succeeded() => *output,
        Err(error) => return Err(error.into()),
    };

    if !output.succeeded() {
        let code = output.code.unwrap_or(-1);
        let message = event_error(&output.stdout)
            .unwrap_or_else(|| preview(output.stderr.trim(), 2000))
            .trim()
            .to_string();
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

    let answer = std::fs::read_to_string(answer_path).map_err(|e| ProviderError::Envelope {
        vendor: VENDOR,
        detail: format!("the CLI wrote no final answer: {e}"),
    })?;
    if answer.trim().is_empty() {
        return Err(ProviderError::Empty(VENDOR));
    }
    Ok(answer)
}

pub(crate) fn write_file(path: &Path, contents: &str) -> Result<(), ProviderError> {
    std::fs::write(path, contents).map_err(|e| ProviderError::Scratch {
        vendor: VENDOR,
        detail: format!("{}: {e}", path.display()),
    })
}

/// A private directory for one Codex invocation.
///
/// **Created exclusively, never reused.** The old name was
/// `bugsleuth-codex-<pid>` in the shared system temp area: entirely
/// predictable, and `create_dir_all` succeeds on a directory that already
/// exists. Any local process could pre-create that path — or leave a link
/// there — and BugSleuth would then write its schema through it, read an
/// `answer.json` it did not produce (forging an entire review), and finally
/// delete whatever the link pointed at.
///
/// `create_dir` without `_all` fails when the path exists, which is what makes
/// this a claim rather than a hope: the directory is ours because creating it
/// is what proved it did not exist. A counter breaks ties within a process and
/// between processes that share a pid across a reboot.
pub(crate) fn scratch_dir() -> Result<PathBuf, ProviderError> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);

    let base = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);

    let mut last = String::new();
    for attempt in 0..64 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("bugsleuth-codex-{pid}-{nanos:08x}-{n}-{attempt}"));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            // Taken, by another invocation or by someone who guessed. Both are
            // answered the same way: try a name nobody has.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                last = e.to_string();
            }
            Err(e) => {
                return Err(ProviderError::Scratch {
                    vendor: VENDOR,
                    detail: e.to_string(),
                });
            }
        }
    }
    Err(ProviderError::Scratch {
        vendor: VENDOR,
        detail: format!(
            "could not create a private working directory in {}: {last}",
            base.display()
        ),
    })
}

/// Removes a directory tree when dropped, so the scratch area is cleaned on a
/// cancelled future and an early `?` as well as on the normal return. A removal
/// that fails is best-effort — it must not mask the provider's result.
pub(crate) struct Cleanup(pub(crate) PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_event_on_stdout_is_preferred_over_empty_stderr() {
        let stdout = r#"{"type":"turn.failed","error":{"message":"rate limited"}}"#;
        assert_eq!(event_error(stdout).as_deref(), Some("rate limited"));
    }

    #[test]
    fn a_plain_error_event_is_also_read() {
        assert_eq!(
            event_error(r#"{"type":"error","message":"boom"}"#).as_deref(),
            Some("boom")
        );
    }

    #[test]
    fn output_that_is_not_events_yields_no_error_rather_than_a_wrong_one() {
        assert_eq!(event_error("not json at all"), None);
        assert_eq!(event_error(r#"{"type":"item.completed"}"#), None);
    }

    #[test]
    fn the_scratch_directory_is_outside_the_reviewed_repository() {
        let dir = scratch_dir().unwrap_or_default();
        assert!(dir.starts_with(std::env::temp_dir()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn each_scratch_directory_is_new_rather_than_reused() {
        // The old name was bugsleuth-codex-<pid> and create_dir_all accepts a
        // directory that already exists, so anything already sitting at that
        // path was silently adopted: its answer.json would have been read back
        // as a review, and it would have been deleted afterwards.
        let a = scratch_dir().unwrap_or_else(|e| panic!("first: {e}"));
        let b = scratch_dir().unwrap_or_else(|e| panic!("second: {e}"));
        assert_ne!(a, b, "two invocations shared a working directory");
        assert!(a.is_dir() && b.is_dir());
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn a_directory_already_sitting_at_the_chosen_name_is_never_adopted() {
        // Proven by taking the name first: whatever scratch_dir returns, it
        // cannot be the one that was already there.
        let planted = scratch_dir().unwrap_or_else(|e| panic!("plant: {e}"));
        let _ = std::fs::write(planted.join("answer.json"), "{\"findings\":[]}");
        let fresh = scratch_dir().unwrap_or_else(|e| panic!("fresh: {e}"));
        assert_ne!(fresh, planted);
        assert!(
            !fresh.join("answer.json").exists(),
            "the new working directory inherited someone else's answer"
        );
        let _ = std::fs::remove_dir_all(&planted);
        let _ = std::fs::remove_dir_all(&fresh);
    }
}
