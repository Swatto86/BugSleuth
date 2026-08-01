//! Codex's working files, and reading its failure events.
//!
//! Codex takes its JSON Schema as a file and can write its final answer to
//! another, so unlike the other adapters it needs somewhere to put them. That
//! somewhere is the system temp area, never inside the repository under review:
//! a review must not leave litter in the thing it is reviewing.

use std::path::{Path, PathBuf};

use crate::error::ProviderError;
use crate::process::preview;

use super::VENDOR;

/// First error carried by a Codex event, if any.
pub(super) fn event_error(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let event: serde_json::Value = serde_json::from_str(line).ok()?;
        match event["type"].as_str()? {
            "turn.failed" => event["error"]["message"].as_str().map(|s| preview(s, 2000)),
            "error" => event["message"].as_str().map(|s| preview(s, 2000)),
            _ => None,
        }
    })
}

pub(super) fn scratch_dir() -> Result<PathBuf, ProviderError> {
    let dir = std::env::temp_dir().join(format!("bugsleuth-codex-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| ProviderError::Scratch {
        vendor: VENDOR,
        detail: e.to_string(),
    })?;
    Ok(dir)
}

pub(super) fn write_file(path: &Path, contents: &str) -> Result<(), ProviderError> {
    std::fs::write(path, contents).map_err(|e| ProviderError::Scratch {
        vendor: VENDOR,
        detail: format!("{}: {e}", path.display()),
    })
}

pub(super) fn not_found() -> ProviderError {
    ProviderError::NotFound {
        vendor: VENDOR,
        hint: "Install the Codex CLI and sign in with `codex login`, or pass an explicit binary \
               path."
            .to_string(),
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
}
