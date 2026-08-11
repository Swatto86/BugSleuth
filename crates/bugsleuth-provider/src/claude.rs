//! Claude Code CLI adapter.
//!
//! Runs `claude --print` non-interactively against a repository. Nothing here
//! trusts the model: a sweep returns `RawFindings`, which cannot reach a report
//! without anchor verification.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::{Lane, RawFindings, finding_schema};
use serde::Deserialize;
use serde_json::Value;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

use discover::resolve_binary;

mod apply;
pub(crate) mod args;
mod discover;
mod envelope;
mod salvage;
mod triage;

pub use apply::{ApplyRequest, apply};
pub use envelope::Usage;
pub use triage::{TriageRequest, TriageResult, triage};

/// Tools a read-only review may use. `--tools` makes this an availability
/// boundary; `--allowedTools` alone only controls permission prompts.
pub(crate) const VENDOR: &str = "claude";

pub(crate) const READ_ONLY_TOOLS: &str = "Read,Glob,Grep";
pub(crate) const READ_ONLY_AGENT_TOOLS: &str = "Read,Glob,Grep,Agent";
pub(crate) const READ_ONLY_ALLOWED: &str = "Read(./**),Glob(./**)";
pub(crate) const READ_ONLY_AGENT_ALLOWED: &str = "Read(./**),Glob(./**),Agent";
pub(crate) const READ_ONLY_DENIED: &str = "Edit,Write,NotebookEdit,Bash,WebFetch,WebSearch";

/// One (model x lane x repository) unit of work.
pub struct ClaudeSweep<'a> {
    /// Repository to review. Becomes the CLI's working directory, so the agent
    /// navigates it itself — these CLIs do their own retrieval and do not need a
    /// hand-assembled context package.
    pub repo: &'a Path,
    pub lane: Lane,
    /// Model alias (`sonnet`, `opus`, `haiku`) or a full model id.
    pub model: &'a str,
    /// Reasoning effort. Empty means the CLI's own default.
    pub effort: &'a str,
    /// Allow Claude's built-in read-only subagents for this sweep.
    pub use_agents: bool,
    /// The assembled review brief, delivered on stdin so a long brief cannot hit
    /// the command-line length limit.
    pub brief: &'a str,
    pub timeout: Duration,
    /// Hard ceiling on agent turns. The main defence against one lane burning
    /// the whole subscription quota.
    pub max_turns: u32,
    /// Explicit binary path, overriding discovery.
    pub binary: Option<&'a str>,
    /// When set, the CLI authenticates with this key instead of the signed-in
    /// subscription session.
    pub api_key: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct SweepResult {
    pub findings: RawFindings,
    pub usage: Option<Usage>,
    /// The CLI's session id, kept so a sweep can be resumed rather than restarted.
    pub session_id: Option<String>,
    pub turns: Option<u32>,
    /// Whether this answer came from a salvaged session rather than the sweep
    /// itself. A salvaged sweep is recovered work, not a clean run: the model
    /// was cut off mid-review and asked afterwards what it had, so the list may
    /// be short in a way an ordinary empty result is not. Presenting the two
    /// identically would let a truncated review read as a thorough one.
    pub salvaged: bool,
}

/// Run one lane sweep.
pub async fn sweep(spec: ClaudeSweep<'_>) -> Result<SweepResult, ProviderError> {
    let _ = spec.lane;
    let ultracode = spec.use_agents && crate::models::supports_ultracode(spec.model);
    let first = invoke::<RawFindings>(Run {
        repo: spec.repo,
        model: spec.model,
        effort: if ultracode { "ultracode" } else { spec.effort },
        prompt: spec.brief,
        schema: finding_schema(),
        available: if spec.use_agents {
            READ_ONLY_AGENT_TOOLS
        } else {
            READ_ONLY_TOOLS
        },
        allowed: if spec.use_agents {
            READ_ONLY_AGENT_ALLOWED
        } else {
            READ_ONLY_ALLOWED
        },
        denied: READ_ONLY_DENIED,
        max_turns: spec.max_turns,
        timeout: spec.timeout,
        binary: spec.binary,
        api_key: spec.api_key,
        session_id: None,
        resume: None,
    })
    .await;

    // A sweep that ran out of turns did the expensive part and then lost it.
    // Ask the same conversation for its answer once before giving up on a lane.
    let outcome = first?;
    let salvaged = outcome.salvaged;

    let findings = crate::json::structured(outcome.structured_result())?;
    Ok(SweepResult {
        findings,
        usage: outcome.usage,
        session_id: outcome.session_id,
        turns: outcome.num_turns,
        salvaged,
    })
}

/// Everything one CLI invocation needs, independent of what it is being asked
/// to do. Shared by sweeps, the severity triage pass and applying fixes, which
/// differ only in prompt, output schema and tool policy.
#[derive(Clone)]
pub(crate) struct Run<'a> {
    pub(crate) repo: &'a Path,
    pub(crate) model: &'a str,
    pub(crate) effort: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) schema: Value,
    pub(crate) available: &'a str,
    pub(crate) allowed: &'a str,
    pub(crate) denied: &'a str,
    pub(crate) max_turns: u32,
    pub(crate) timeout: Duration,
    pub(crate) binary: Option<&'a str>,
    pub(crate) api_key: Option<&'a str>,
    /// The id assigned to a new conversation so a killed CLI can be resumed.
    pub(crate) session_id: Option<String>,
    /// Continue an existing conversation instead of starting one. Only the
    /// salvage path uses this: everything else must start clean, or a sweep
    /// would inherit whatever the last one was thinking.
    pub(crate) resume: Option<&'a str>,
}

/// Run the CLI, and recover an interrupted or missing answer once.
///
/// The retry lives here rather than in each caller because every one of them
/// has the same problem: the expensive work is done inside a conversation that
/// then fails to answer. Sweeps, the severity triage pass and applying fixes all
/// lost whole invocations to it — the triage pass most often, and it has no
/// repository to read at all.
pub(crate) async fn invoke<T: serde::de::DeserializeOwned>(
    mut run: Run<'_>,
) -> Result<ResultEnvelope, ProviderError> {
    crate::models::validate_effort(VENDOR, run.model, run.effort).await?;
    if run.resume.is_none() && run.session_id.is_none() {
        run.session_id = Some(salvage::new_session_id());
    }
    let recovery = run.clone();
    salvage::recover::<T>(invoke_once(run).await, recovery).await
}

pub(super) async fn invoke_once(run: Run<'_>) -> Result<ResultEnvelope, ProviderError> {
    let binary = match run.binary {
        Some(path) => PathBuf::from(path),
        None => resolve_binary().ok_or_else(not_found)?,
    };
    let binary = binary.to_string_lossy().into_owned();

    let args = build_args(&run);
    let mut env: Vec<(String, String)> = run
        .api_key
        .map(|key| vec![("ANTHROPIC_API_KEY".to_string(), key.to_string())])
        .unwrap_or_default();
    if run.resume.is_some() {
        env.push((
            "CLAUDE_CODE_RESUME_INTERRUPTED_TURN".to_string(),
            "1".to_string(),
        ));
    }

    let output = process::run(Invocation {
        binary: &binary,
        args: &args,
        cwd: run.repo,
        stdin: Some(run.prompt.as_bytes()),
        env: &env,
        timeout: run.timeout,
        what: "claude CLI",
    })
    .await?;

    if !output.succeeded() {
        let code = output.code.unwrap_or(-1);
        // A turn-budget exhaustion is not an ordinary failure: the review may
        // already be finished inside the conversation this names, so it is
        // reported distinctly rather than collapsed into "exited non-zero".
        if let Some(session) = salvage::exhausted_session(&output.stdout) {
            return Err(ProviderError::TurnsExhausted {
                vendor: VENDOR,
                session: Some(session),
            });
        }
        // stderr first, then stdout. `--output-format json` means this CLI puts
        // its own account of a failure in the envelope on stdout, and reading
        // only stderr reported "produced no diagnostic output" for two real
        // sweeps that had almost certainly explained themselves. Codex already
        // did it this way round; Kilo was fixed for the same reason.
        let message = match preview(output.stderr.trim(), 2000) {
            text if !text.is_empty() => text,
            _ => envelope::failure_text(&output.stdout),
        };
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

    let stdout = output.stdout.trim();
    if stdout.is_empty() {
        return Err(ProviderError::Empty(VENDOR));
    }
    envelope::parse(stdout)
}

/// Check that the CLI exists and can run, returning its version.
///
/// Cheap and free: `--version` starts no model. This exists because the
/// alternative — discovering a missing or signed-out CLI when the first real
/// sweep fails — wastes the wait the user already spent on earlier lanes.
///
/// Note what this does *not* prove: `--version` succeeds for a CLI that is
/// installed but not signed in. Authentication is only observable by making a
/// real call, so a run still has to handle an auth failure at sweep time.
pub async fn probe() -> Result<String, ProviderError> {
    let binary = resolve_binary().ok_or_else(not_found)?;
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &["--version".to_string()],
        cwd: Path::new("."),
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(30),
        what: "claude CLI",
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

/// The CLI is missing. The message names the exact install and sign-in steps,
/// because "not found" on its own sends the reader hunting.
fn not_found() -> ProviderError {
    ProviderError::NotFound {
        vendor: VENDOR,
        hint: "Install it with `npm install -g @anthropic-ai/claude-code` and sign in by running                `claude` once, or pass an explicit binary path."
            .to_string(),
    }
}

/// Build the non-interactive argv.
///
/// `--safe-mode` disables every customization the machine or the repository
/// under review would otherwise inject — CLAUDE.md, hooks, skills, MCP servers,
/// custom agents. Without it, reviewing a repository would execute that
/// repository's hooks, and the review's behaviour would silently depend on
/// whatever is in the developer's global config. Authentication is unaffected,
/// so the signed-in subscription session still applies.
use args::build_args;

#[derive(Debug, Deserialize)]
pub(crate) struct ResultEnvelope {
    /// Set by us, never by the CLI: true when this answer was recovered from a
    /// session that ran out of turns rather than produced by the run itself.
    #[serde(skip)]
    pub(crate) salvaged: bool,
    #[serde(default)]
    pub(crate) result: Value,
    #[serde(default)]
    pub(crate) structured_output: Option<Value>,
    #[serde(default)]
    pub(crate) is_error: bool,
    #[serde(default)]
    pub(crate) subtype: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) num_turns: Option<u32>,
    #[serde(default)]
    pub(crate) usage: Option<Usage>,
}

/// Prove the machine holds a Claude session, by using it.
///
/// `--print` with no tools and no schema: the cheapest thing this CLI can be
/// asked to do that still requires being signed in.
///
/// `--safe-mode` disables every customization the machine or the surrounding
/// repository would inject — CLAUDE.md, hooks, skills, MCP servers — exactly as
/// a real review does. The probe is invoked before any sweep, so without it a
/// run from an untrusted checkout would load that checkout's instructions or
/// hooks as part of the supposedly harmless one-word check. The probe also runs
/// from a private empty directory rather than the caller's working directory, so
/// a future CLI change cannot make it depend on the files around it.
pub async fn signin(binary: Option<&str>) -> crate::signin::SignIn {
    let binary = match binary {
        Some(path) => PathBuf::from(path),
        None => match resolve_binary() {
            Some(path) => path,
            None => return crate::signin::SignIn::Failed(not_found().to_string()),
        },
    };
    let args: Vec<String> = ["--print", "--safe-mode", "--output-format", "text"]
        .iter()
        .map(|a| (*a).to_string())
        .collect();
    let sandbox = empty_probe_dir();
    let cwd = sandbox.as_deref().unwrap_or_else(|| Path::new("."));
    let result = crate::signin::one_shot(
        &binary.to_string_lossy(),
        &args,
        cwd,
        &[],
        "claude",
        str::to_string,
    )
    .await;
    if let Some(dir) = sandbox {
        let _ = std::fs::remove_dir_all(dir);
    }
    result
}

/// A private empty directory for the sign-in probe, or `None` if one cannot be
/// made. `create_dir` rather than `create_dir_all` so a collision proves the
/// path was not ours.
fn empty_probe_dir() -> Option<PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-claude-signin-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).ok().map(|_| dir)
}

#[cfg(test)]
#[path = "claude/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "claude/permission_tests.rs"]
mod permission_tests;

#[cfg(test)]
#[path = "claude/signin_probe_tests.rs"]
mod signin_probe_tests;
