//! Handing the fix prompt to Codex, with write access to the real repository.
//!
//! `--sandbox workspace-write` rather than `read-only`, pointed at the user's
//! own checkout rather than a throwaway worktree. Where the other vendors
//! confine writes with a tool allowlist this tool sets, Codex's confinement is
//! the CLI's own sandbox — which is a stronger mechanism where the platform
//! supports it and a weaker claim to make from here, because BugSleuth cannot
//! observe it. What *was* observed on Windows: the CLI accepts the flag without
//! complaint. So this file claims only what it sets, and the safety story it
//! relies on is the same one every vendor's apply relies on — git, below.
//!
//! What makes an apply recoverable either way is git: the engine refuses to
//! start unless the working tree is clean, so everything this does shows up in
//! `git status` afterwards and can be thrown away with one command.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;
use crate::process::{self, Invocation};

use super::scratch::{Cleanup, finish, scratch_dir};
use super::{SHARED_FLAGS, VENDOR, not_found};

/// Apply the fixes described in `prompt`, returning the model's own account.
///
/// No output schema: the answer is prose for a person, and constraining it would
/// spend a turn to say less. Codex writes its final message to a file, so
/// nothing has to be scraped out of the event stream.
pub async fn apply(
    repo: &Path,
    model: &str,
    effort: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, ProviderError> {
    // The accepted reasoning efforts belong to the model, not the CLI, so an
    // effort forwarded to `model_reasoning_effort` is validated against the
    // model's catalogue before anything is spent.
    crate::models::validate_effort(VENDOR, model, effort).await?;

    let binary = super::binary_path().ok_or_else(not_found)?;
    let scratch = scratch_dir()?;
    // Removed on every exit — normal return, early `?`, and a cancelled future
    // dropped at its await point.
    let _scratch = Cleanup(scratch.clone());
    let answer_path = scratch.join("answer.md");

    let args = build_args(model, effort, &answer_path);
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &args,
        cwd: repo,
        stdin: Some(prompt.as_bytes()),
        env: &[],
        timeout,
        what: "codex CLI",
    })
    .await;

    finish(output, &answer_path)
}

/// The argv for one write-capable invocation.
///
/// Built from the same [`SHARED_FLAGS`] the sign-in check uses, so a probe that
/// passes is evidence about the invocation that does the work.
fn build_args(model: &str, effort: &str, answer: &Path) -> Vec<String> {
    let mut args: Vec<String> = SHARED_FLAGS.iter().map(|s| (*s).to_string()).collect();
    // Measured against the real CLI, not assumed: with `--ignore-user-config`
    // this CLI refuses every patch as "writing is blocked by read-only
    // sandbox", whatever `--sandbox` says — and `-c sandbox_mode=…` and
    // `-c approval_policy=…` do not restore it either. So an invocation that
    // has to write cannot also ignore the machine's configuration, and the
    // honest choice is to keep the writing.
    //
    // `--ignore-rules` stays. That is the flag which keeps the repository —
    // whose findings prompted this, and whose contents are untrusted — from
    // supplying its own execution policy, and it does not interfere.
    args.retain(|flag| *flag != "--ignore-user-config");
    // No session file for a run that edits someone's repository: there is
    // nothing here worth resuming, and a persisted transcript of the fix is one
    // more copy of the code lying around.
    args.push("--ephemeral".into());
    args.push("--json".into());
    args.push("--sandbox".into());
    args.push("workspace-write".into());
    args.push("--output-last-message".into());
    args.push(answer.to_string_lossy().into_owned());

    let model = model.trim();
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.to_string());
    }
    // Codex has no effort flag; it is a config key set for this invocation only.
    let effort = effort.trim();
    if !effort.is_empty() {
        args.push("-c".into());
        args.push(format!("model_reasoning_effort=\"{effort}\""));
    }
    // A bare `-` tells Codex to read the prompt from stdin, so a long handoff
    // cannot hit the command-line length limit.
    args.push("-".into());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(model: &str, effort: &str) -> Vec<String> {
        build_args(model, effort, Path::new("answer.md"))
    }

    #[test]
    fn applying_is_granted_workspace_write_and_never_asks() {
        let args = argv("", "");
        let sandbox = args
            .iter()
            .position(|a| a == "--sandbox")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        assert_eq!(sandbox, Some("workspace-write"), "{args:?}");
        // Non-interactive: an apply that stopped to ask would hang until it
        // timed out, with the repository half-changed.
        let approval = args
            .iter()
            .position(|a| a == "--ask-for-approval")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        assert_eq!(approval, Some("never"), "{args:?}");
    }

    #[test]
    fn a_write_capable_run_keeps_the_machines_configuration_but_not_the_repositorys_rules() {
        let args = argv("", "");
        // Both are deliberate and were measured against the real CLI: dropping
        // the first is what makes writing work at all, keeping the second is
        // what stops the repository choosing its own execution policy.
        assert!(
            !args.iter().any(|a| a == "--ignore-user-config"),
            "writing is refused when the machine's config is ignored: {args:?}"
        );
        assert!(args.iter().any(|a| a == "--ignore-rules"), "{args:?}");
    }

    #[test]
    fn the_prompt_is_read_from_stdin_rather_than_argv() {
        assert_eq!(argv("", "").last().map(String::as_str), Some("-"));
    }

    #[test]
    fn an_empty_model_or_effort_is_omitted_rather_than_passed_blank() {
        let args = argv("  ", "  ");
        assert!(!args.iter().any(|a| a == "-m"), "{args:?}");
        assert!(!args.iter().any(|a| a == "-c"), "{args:?}");
    }

    #[test]
    fn a_named_model_and_effort_reach_the_cli() {
        let args = argv("gpt-5.6-codex", "high");
        assert!(args.windows(2).any(|w| w == ["-m", "gpt-5.6-codex"]));
        assert!(
            args.windows(2)
                .any(|w| w == ["-c", "model_reasoning_effort=\"high\""])
        );
    }
}
