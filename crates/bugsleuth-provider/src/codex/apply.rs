//! Handing the fix prompt to Codex, with write access to the real repository.
//!
//! `workspace-write` rather than `read-only`, and pointed at the user's own
//! checkout rather than a throwaway worktree. What makes that recoverable is
//! git: the engine refuses to start unless the working tree is clean, so
//! everything this does is visible in `git status` afterwards.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;

use super::{Invoke, Sandbox, invoke_text};

/// Apply the fixes described in `prompt`, returning the model's own account.
///
/// No output schema: the answer is prose for a person, and constraining it would
/// spend a turn to say less. Codex still writes its final message to a file, so
/// nothing has to be scraped out of the event stream.
pub async fn apply(
    repo: &Path,
    model: &str,
    effort: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, ProviderError> {
    invoke_text(Invoke {
        dir: repo,
        model,
        effort,
        brief: prompt,
        timeout,
        binary: None,
        schema: serde_json::Value::Null,
        sandbox: Sandbox::WorkspaceWrite,
    })
    .await
}
