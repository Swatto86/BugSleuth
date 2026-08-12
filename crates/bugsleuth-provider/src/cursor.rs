//! Cursor Agent CLI adapter.
//!
//! The fifth vendor. Users reach it by typing `agent` in a terminal; BugSleuth
//! names it `cursor` so a model spec reads `cursor:composer-2.5` rather than
//! colliding with the word "agent".
//!
//! **Read-only by flag.** `--mode ask` is Cursor's Ask mode: read-only
//! exploration without edits. That is the per-invocation boundary Claude gets
//! from a tool allowlist and Codex from `--sandbox read-only`.
//!
//! **No ignore-rules flag.** Project instruction files (`.cursor`, `AGENTS.md`,
//! `.cursorrules`) would otherwise brief the reviewer. Sweeps therefore run in
//! a throwaway worktree whose instruction files have already been stripped —
//! [`CursorSweep`] takes a `worktree`, not a `repo`.
//!
//! **No output-schema flag.** The required JSON shape is described in the brief
//! and validated afterwards, which is strictly weaker than a schema the CLI
//! enforces.
//!
//! **Prompt as a file.** `-p` takes an argv string; a 12 KB brief does not fit
//! through a Windows `cmd.exe` shim. See [`brief_file`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::RawFindings;
use serde_json::Value;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

mod apply;
mod brief_file;
mod catalogue;
mod discover;
mod signin;
pub use apply::apply;
pub use signin::{signin, signin_for};

pub(crate) const VENDOR: &str = "cursor";

/// One read-only sweep of a throwaway checkout.
pub struct CursorSweep<'a> {
    /// A throwaway checkout whose instruction files have been stripped.
    pub worktree: &'a Path,
    /// Model id as `agent models` lists it. Empty means the CLI's default.
    pub model: &'a str,
    /// The brief. Must already describe the required JSON shape.
    pub brief: &'a str,
    pub timeout: Duration,
    /// Explicit CLI path for tests; real runs use discovery.
    pub binary: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct CursorResult {
    pub findings: RawFindings,
}

/// Where the Cursor Agent CLI lives, if it is installed.
#[must_use]
pub fn binary_path() -> Option<PathBuf> {
    discover::binary_path()
}

/// The flags every read-only Cursor invocation shares.
///
/// `--mode ask` is the write boundary. `--trust` skips the workspace prompt in
/// print mode. `--output-format text` yields the assistant's own words for the
/// JSON extractor.
const BASE_FLAGS: [&str; 6] = ["-p", "--mode", "ask", "--trust", "--output-format", "text"];

pub async fn sweep(spec: CursorSweep<'_>) -> Result<CursorResult, ProviderError> {
    let launch = match spec.binary {
        Some(path) => discover::Launch {
            binary: PathBuf::from(path),
            prefix: Vec::new(),
        },
        None => discover::resolve().ok_or_else(not_found)?,
    };
    let brief = brief_file::BriefFile::write_in(spec.worktree, spec.brief)?;
    let args = build_args(&launch.prefix, &spec, &brief);

    let output = process::run(Invocation {
        binary: &launch.binary.to_string_lossy(),
        args: &args,
        cwd: spec.worktree,
        stdin: None,
        env: &[],
        timeout: spec.timeout,
        what: "cursor agent CLI",
    })
    .await?;

    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 500),
        });
    }

    let findings = crate::json::structured(&Value::String(output.stdout.clone()))?;
    Ok(CursorResult { findings })
}

/// The argv for one sweep.
fn build_args(
    prefix: &[String],
    spec: &CursorSweep<'_>,
    _brief: &brief_file::BriefFile,
) -> Vec<String> {
    let mut args = prefix.to_vec();
    args.extend(BASE_FLAGS.iter().map(|flag| (*flag).to_string()));

    // Pin the workspace explicitly. The process cwd is also the worktree, but
    // the flag is what Cursor documents for headless use, and the sign-in probe
    // shares this builder — both must agree.
    args.push("--workspace".into());
    args.push(spec.worktree.to_string_lossy().into_owned());

    let model = spec.model.trim();
    if !model.is_empty() {
        args.push("--model".into());
        args.push(model.to_string());
    }

    args.push(brief_file::review_pointer());
    args
}

/// Whether the CLI can be started at all, and which version answered.
pub async fn probe() -> Result<String, ProviderError> {
    let launch = discover::resolve().ok_or_else(not_found)?;
    let mut args = launch.prefix.clone();
    args.push("-v".into());
    let output = process::run(Invocation {
        binary: &launch.binary.to_string_lossy(),
        args: &args,
        cwd: Path::new("."),
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(60),
        what: "cursor agent CLI",
    })
    .await?;

    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 500),
        });
    }
    Ok(output
        .stdout
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("unknown")
        .trim()
        .to_string())
}

fn not_found() -> ProviderError {
    ProviderError::NotFound {
        vendor: VENDOR,
        hint: "install the Cursor Agent CLI (`agent`) and sign in with `agent login`".to_string(),
    }
}

/// Ask the installed CLI which models this account can use.
///
/// Falls back to an empty list on any failure so the picker can offer a
/// typed-in id rather than looking identical to "no models exist".
pub async fn list_model_ids() -> Vec<String> {
    let Some(launch) = discover::resolve() else {
        return Vec::new();
    };
    let mut args = launch.prefix.clone();
    args.push("models".into());
    let output = process::run(Invocation {
        binary: &launch.binary.to_string_lossy(),
        args: &args,
        cwd: &std::env::temp_dir(),
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(60),
        what: "cursor agent models",
    })
    .await;
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.succeeded() {
        return Vec::new();
    }
    catalogue::parse(&output.stdout)
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

#[cfg(test)]
#[path = "cursor/tests.rs"]
mod tests;
