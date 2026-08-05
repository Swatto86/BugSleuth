//! Handing the fix prompt to Kilo, in the real repository.
//!
//! The one job where Kilo's awkwardness stops mattering. A sweep needs a schema
//! it cannot be given and a read-only mode it does not have, which is why sweeps
//! are confined to a throwaway worktree. Applying fixes needs neither: the task
//! *is* to write, and the answer is prose rather than a structure. So this is the
//! one invocation where Kilo is used exactly as its own users use it.
//!
//! What keeps it recoverable is the same thing as for the other two vendors —
//! the engine refuses to start unless the working tree is clean, so everything
//! done here shows up in `git status`.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;
use crate::process::{self, Invocation};

use super::{BASE_FLAGS, assistant_text_or_error, discover, not_found};

/// Apply the fixes described in `prompt`, returning the model's own account.
pub async fn apply(
    repo: &Path,
    model: &str,
    effort: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, ProviderError> {
    let binary = discover::resolve_binary().ok_or_else(not_found)?;

    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &build_args(repo, model, effort),
        cwd: repo,
        stdin: Some(prompt.as_bytes()),
        env: &[],
        timeout,
        what: "kilo CLI",
    })
    .await?;

    assistant_text_or_error(&output)
}

/// The sweep's flags without `--agent ask`.
///
/// Deliberate: `ask` is the read-oriented agent, and this invocation has to
/// edit files. Leaving the agent unset uses whichever one Kilo is configured
/// with, which is how its own users run it.
fn build_args(repo: &Path, model: &str, effort: &str) -> Vec<String> {
    let mut args: Vec<String> = BASE_FLAGS.iter().map(|s| (*s).to_string()).collect();

    // Pinned explicitly as well as via the process's cwd: Kilo resolves some
    // paths from `--dir` rather than the process cwd, and a mismatch would have
    // it edit a different tree than the one the report is about.
    args.push("--dir".into());
    args.push(repo.to_string_lossy().into_owned());

    let model = model.trim();
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.to_string());
    }
    let effort = effort.trim();
    if !effort.is_empty() {
        args.push("--variant".into());
        args.push(effort.to_string());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_does_not_use_the_read_oriented_agent() {
        // `--agent ask` is the sweep's, and it cannot edit files. Passing it
        // here would leave the button apparently working and nothing changed.
        let args = build_args(Path::new("/tmp/repo"), "", "");
        assert!(!args.iter().any(|a| a == "ask"));
        // The safety flags a sweep carries are still here.
        assert!(args.iter().any(|a| a == "--pure"));
        assert!(args.iter().any(|a| a == "--auto"));
        let dir = args
            .iter()
            .position(|a| a == "--dir")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        assert_eq!(dir, Some("/tmp/repo"));
    }
}
