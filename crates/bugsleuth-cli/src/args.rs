//! Turning CLI arguments into validated values.
//!
//! Split from `main.rs` at the hard line cap, along the seam already there:
//! everything here refuses or converts before any quota is spent, while
//! `main.rs` is the command wiring that spends it. Two of these encode the same
//! rule — a capability only the Claude adapter has — and both refuse rather
//! than accepting an option they would then silently ignore.

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Canonicalize a path, then strip Windows' extended-length `\\?\` prefix.
///
/// `canonicalize` returns that prefix on Windows and most tools handle it, but
/// `git` does not: it fails with "could not create leading directories" when
/// asked to make a worktree under such a path. Stripping it costs nothing on
/// paths short enough to matter, and every path here is one a user typed.
pub(crate) fn real_path(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    // A UNC network path must keep its `\\server\share` form rather than be
    // truncated to a relative `UNC\server\share`; one shared conversion.
    Ok(bugsleuth_engine::git_path(&canonical))
}

/// The key is only ever read from the environment. Passing it as an argument
/// would put it in shell history and in the process list.
pub(crate) fn api_key(requested: bool) -> Result<Option<String>> {
    if !requested {
        return Ok(None);
    }
    std::env::var("ANTHROPIC_API_KEY")
        .map(Some)
        .map_err(|_| anyhow::anyhow!("--use-api-key was given but ANTHROPIC_API_KEY is not set"))
}

/// Refuse API-key mode for a provider the key cannot reach.
///
/// `--use-api-key` supplies `ANTHROPIC_API_KEY`, and only the Claude adapter
/// takes one — Codex and Kilo use the session configured in their own CLI. The
/// flag was accepted for every model, so a Codex or Kilo sweep read the key and
/// then ran without it: the signed-in account the user had explicitly asked not
/// to use was used, and charged, with nothing said.
///
/// A mixed `run` is allowed, because the key really does apply to its Claude
/// units and to the Claude triage pass; only a run that reaches no Claude call
/// at all is refused.
pub(crate) fn validate_api_key_target(requested: bool, uses_claude: bool) -> Result<()> {
    if requested && !uses_claude {
        anyhow::bail!(
            "--use-api-key supplies ANTHROPIC_API_KEY and is only supported by Claude; Codex \
             and Kilo use the session configured in their own CLI"
        );
    }
    Ok(())
}

pub(crate) fn claude_turn_budget(
    requested: Option<u32>,
    uses_claude: bool,
    default: u32,
) -> Result<u32> {
    if requested.is_some() && !uses_claude {
        anyhow::bail!(
            "--max-turns is only supported by Claude; Codex and Kilo use a per-invocation \
             --timeout-secs limit plus one bounded recovery"
        );
    }
    Ok(requested.unwrap_or(default))
}

pub(crate) fn sweep_start_message(
    repo: &Path,
    lane: bugsleuth_domain::Lane,
    model: &str,
    timeout_secs: u64,
) -> String {
    format!(
        "Starting {lane} sweep of {} with {model} (timeout {timeout_secs}s)…",
        repo.display()
    )
}
