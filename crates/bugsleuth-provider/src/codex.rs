//! Codex CLI adapter.
//!
//! The second vendor, and the reason BugSleuth exists in the shape it does: two
//! models from the same family share blind spots, so a review that only ever
//! asks Claude is not the cross-vendor audit the tool promises.
//!
//! Codex differs from Claude in three ways that matter here, all absorbed
//! inside this file so the layer above sees one uniform result:
//!
//! - Its JSON Schema is passed as a **file path**, not inline text.
//! - Its final answer can be written straight to a file with
//!   `--output-last-message`, which avoids parsing its event stream at all.
//! - Its sandbox is a first-class flag (`--sandbox read-only`) rather than a
//!   tool allowlist.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::RawFindings;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

mod apply;
mod discover;
pub(crate) mod scratch;
mod sweep;

pub use apply::apply;
pub use sweep::sweep;

pub(crate) const VENDOR: &str = "codex";

pub struct CodexSweep<'a> {
    pub repo: &'a Path,
    /// Model id, e.g. `gpt-5.6-codex`. Empty means the CLI's own default.
    pub model: &'a str,
    /// Reasoning effort. Empty means the CLI's own default.
    pub effort: &'a str,
    pub brief: &'a str,
    pub timeout: Duration,
    pub binary: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct CodexResult {
    pub findings: RawFindings,
    pub salvaged: bool,
}

/// The flags every Codex invocation carries, whatever it is for.
///
/// One list, because a sign-in check that invokes the CLI differently from the
/// work is not checking the work — it can pass while every real run fails, or
/// fail while runs are fine. The first sign-in probe dropped
/// `--skip-git-repo-check` and reported a working Codex as unusable.
///
/// `--ignore-user-config` and `--ignore-rules` are the security-relevant pair:
/// neither the machine's own configuration nor the repository's rules may
/// change what the model is told to do. The write-capable path has to drop the
/// first of those to write at all — see [`apply`].
pub(crate) const SHARED_FLAGS: [&str; 8] = [
    "--ask-for-approval",
    "never",
    "exec",
    "--skip-git-repo-check",
    "--ignore-user-config",
    "--ignore-rules",
    "--color",
    "never",
];

fn signin_args() -> Vec<String> {
    SHARED_FLAGS
        .iter()
        .chain(["--sandbox", "read-only", "-"].iter())
        .map(|arg| (*arg).to_string())
        .collect()
}

/// Where the Codex CLI is, if it is installed.
///
/// Exposed for the model catalogue, which needs the binary without wanting
/// anything else this module does. Mirrors `kilo::binary_path`.
#[must_use]
pub fn binary_path() -> Option<PathBuf> {
    discover::resolve_binary()
}

fn not_found() -> ProviderError {
    ProviderError::NotFound {
        vendor: VENDOR,
        hint:
            "Install the Codex CLI and sign in with `codex login`, or pass an explicit binary path."
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
        timeout: Duration::from_secs(30),
        what: "codex CLI",
    })
    .await?;

    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 500),
        });
    }
    Ok(output.stdout.trim().to_string())
}

/// Prove the machine holds a ChatGPT session with a fixed, non-repository prompt.
pub async fn signin() -> crate::signin::SignIn {
    let Some(binary) = discover::resolve_binary() else {
        return crate::signin::SignIn::Failed("the codex CLI could not be found".to_string());
    };
    crate::signin::one_shot(
        &binary.to_string_lossy(),
        &signin_args(),
        Path::new("."),
        &[],
        "codex",
        str::to_string,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signin_ignores_host_and_repository_instructions() {
        let args = signin_args();
        for required in [
            "--skip-git-repo-check",
            "--ignore-user-config",
            "--ignore-rules",
            "--sandbox",
            "read-only",
        ] {
            assert!(args.iter().any(|arg| arg == required), "missing {required}");
        }
        assert_eq!(args.last().map(String::as_str), Some("-"));
    }
}

#[cfg(test)]
#[path = "codex/capability_tests.rs"]
mod capability_tests;
