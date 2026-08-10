//! Handing the fix prompt to Kilo, with write access to the real repository.
//!
//! Kilo has no per-invocation confinement: its permissions come from the user's
//! own `kilo.jsonc`, and `--auto` only auto-approves what that file marks `ask`.
//! A sweep answers that by never being pointed at the repository under review —
//! it gets a throwaway worktree with the repository's own configuration stripped
//! out of it. An apply cannot: editing the real checkout is the whole point.
//!
//! So the two things that *can* be checked are checked here, before anything is
//! spent:
//!
//! - **The repository must not configure Kilo.** Measured against the real CLI:
//!   a `kilo.jsonc` sitting in the working directory rewrites the resolved
//!   permissions of an agent named on the command line — an injected project
//!   config turned `edit` and `bash` back to `allow` for an agent the machine's
//!   own configuration restricts. A repository that ships one would be choosing
//!   its own permissions, so an apply into that repository is refused rather
//!   than run under rules the repository wrote.
//! - **The agent must be able to edit.** Running the read-oriented agent would
//!   spend a whole paid run to change nothing.
//!
//! Beyond that, the safety story is git, not a sandbox — the engine refuses to
//! start unless the working tree is clean, so everything this does is visible in
//! `git status` and can be thrown away with one command.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;
use crate::process::{self, Invocation};

use super::preflight;
use super::{BASE_FLAGS, VENDOR, assistant_text_or_error, discover, not_found, operation_guard};

/// Configuration a repository can use to redefine the agent an apply runs as.
///
/// The same names the sweep isolator removes from a throwaway worktree. Here
/// they cannot be removed — it is the user's own checkout — so their presence
/// refuses the run instead.
const REPOSITORY_CONFIG: &[&str] = &["kilo.jsonc", "kilo.json", ".kilo", ".kilocode"];

/// Apply the fixes described in `prompt`, returning the model's own account.
pub async fn apply(
    repo: &Path,
    model: &str,
    effort: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, ProviderError> {
    if let Some(reason) = repository_configures_kilo(repo) {
        return Err(ProviderError::CapabilityUnavailable {
            vendor: VENDOR,
            capability: "apply",
            reason,
        });
    }
    if let Some(reason) = preflight::apply_gap() {
        return Err(ProviderError::CapabilityUnavailable {
            vendor: VENDOR,
            capability: "apply",
            reason,
        });
    }

    let binary = discover::resolve_binary().ok_or_else(not_found)?;

    // Held for the same reason a sweep holds it: two Kilo processes share one
    // credential store. Taken after the local checks, so a refused apply never
    // makes a queued sweep wait.
    let _operation = operation_guard().await;

    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &build_args(repo, model, effort),
        cwd: repo,
        stdin: Some(prompt.as_bytes()),
        env: &apply_environment(),
        timeout,
        what: "kilo CLI",
    })
    .await?;

    assistant_text_or_error(&output)
}

/// Why this repository cannot be applied into, or `None` when it is ordinary.
fn repository_configures_kilo(repo: &Path) -> Option<String> {
    let found = REPOSITORY_CONFIG
        .iter()
        .find(|name| repo.join(name).exists())?;
    Some(format!(
        "{found} in this repository would redefine the agent the fixes are applied by, so the \
         repository would be choosing its own permissions. Apply the generated handoff manually, \
         or move that configuration out of the repository first."
    ))
}

/// Third-party skills are code this tool did not choose, exactly as they are for
/// a sweep.
fn apply_environment() -> [(String, String); 1] {
    [(
        "KILO_DISABLE_EXTERNAL_SKILLS".to_string(),
        "true".to_string(),
    )]
}

/// The argv for one write-capable invocation.
///
/// The agent is named rather than left to Kilo's default, because "the default"
/// is whatever the configuration in scope says it is — and naming it is what
/// makes [`preflight::apply_gap`] a check on the agent that actually runs.
fn build_args(repo: &Path, model: &str, effort: &str) -> Vec<String> {
    let mut args: Vec<String> = BASE_FLAGS
        .iter()
        .chain(["--agent", preflight::APPLY_AGENT].iter())
        .map(|s| (*s).to_string())
        .collect();

    // Pinned explicitly as well as through the process working directory: Kilo
    // resolves some paths from `--dir` rather than from the cwd, and an apply
    // pointed at a different tree than the one the engine measures afterwards
    // would report someone else's changes as this run's.
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

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bugsleuth-kilo-apply-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch directory");
        dir
    }

    #[test]
    fn an_ordinary_repository_is_applied_into() {
        let dir = scratch("plain");
        std::fs::write(dir.join("README.md"), "hello").expect("seed file");
        assert_eq!(repository_configures_kilo(&dir), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_repository_carrying_kilo_configuration_is_refused() {
        for name in REPOSITORY_CONFIG {
            let dir = scratch(&name.replace('.', ""));
            let planted = dir.join(name);
            if name.starts_with('.') && !name.contains("json") {
                std::fs::create_dir_all(&planted).expect("plant config directory");
            } else {
                std::fs::write(&planted, "{}").expect("plant config file");
            }
            let refusal = repository_configures_kilo(&dir)
                .unwrap_or_else(|| panic!("{name} in the repository was not noticed"));
            assert!(refusal.contains(name), "{refusal}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn a_repository_that_configures_kilo_is_refused_before_the_cli_starts() {
        // The refusal has to come before a process exists: a run that started
        // and then complained would already have handed the repository's own
        // rules to the CLI.
        let dir = scratch("preflight");
        std::fs::write(dir.join("kilo.jsonc"), "{}").expect("plant config");
        let error = apply(&dir, "", "", "fix it", Duration::from_secs(5))
            .await
            .expect_err("an apply into a self-configuring repository must be refused");
        let shown = error.to_string();
        assert!(shown.contains("kilo.jsonc"), "{shown}");
        assert!(shown.contains("apply is unavailable"), "{shown}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_run_names_its_agent_and_its_directory() {
        let repo = Path::new("/tmp/example");
        let args = build_args(repo, "kilo/openai/gpt-5.6-sol", "high");
        assert!(
            args.windows(2)
                .any(|w| w == ["--agent", preflight::APPLY_AGENT]),
            "{args:?}"
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--dir" && w[1] == repo.to_string_lossy()),
            "{args:?}"
        );
        assert!(
            args.windows(2)
                .any(|w| w == ["-m", "kilo/openai/gpt-5.6-sol"]),
            "{args:?}"
        );
        assert!(
            args.windows(2).any(|w| w == ["--variant", "high"]),
            "{args:?}"
        );
    }

    #[test]
    fn an_apply_never_runs_as_the_read_only_sweep_agent() {
        // The two agents exist for opposite reasons. Running the sweep's one
        // here would spend a paid run and change nothing.
        assert_ne!(preflight::APPLY_AGENT, preflight::SWEEP_AGENT);
        let args = build_args(Path::new("."), "", "");
        assert!(
            !args
                .windows(2)
                .any(|w| w == ["--agent", preflight::SWEEP_AGENT]),
            "{args:?}"
        );
    }
}
