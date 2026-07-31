//! Claude Code CLI adapter.
//!
//! Runs `claude --print` non-interactively against a repository. Nothing here
//! trusts the model: a sweep returns `RawFindings`, which cannot reach a report
//! without anchor verification, and a proof attempt returns the model's own
//! account of what it did, which is checked by re-running the tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::{Lane, RawFindings, finding_schema};
use serde::Deserialize;
use serde_json::Value;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

use discover::resolve_binary;

mod discover;
mod envelope;
mod prove;

pub use envelope::Usage;
pub use prove::{ProveRequest, ProveResult, prove};

/// Tools a read-only review may use. An explicit allowlist rather than
/// `--dangerously-skip-permissions`: a sweep that *cannot* write is a far
/// stronger guarantee than one merely asked not to.
pub(crate) const VENDOR: &str = "claude";

const READ_ONLY_TOOLS: &str = "Read,Glob,Grep";
const READ_ONLY_DENIED: &str = "Edit,Write,NotebookEdit,Bash,WebFetch,WebSearch";

/// One (model x lane x repository) unit of work.
pub struct ClaudeSweep<'a> {
    /// Repository to review. Becomes the CLI's working directory, so the agent
    /// navigates it itself — these CLIs do their own retrieval and do not need a
    /// hand-assembled context package.
    pub repo: &'a Path,
    pub lane: Lane,
    /// Model alias (`sonnet`, `opus`, `haiku`) or a full model id.
    pub model: &'a str,
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
}

/// Run one lane sweep.
pub async fn sweep(spec: ClaudeSweep<'_>) -> Result<SweepResult, ProviderError> {
    let _ = spec.lane;
    let outcome = invoke(Run {
        repo: spec.repo,
        model: spec.model,
        prompt: spec.brief,
        schema: finding_schema(),
        allowed: READ_ONLY_TOOLS,
        denied: READ_ONLY_DENIED,
        max_turns: spec.max_turns,
        timeout: spec.timeout,
        binary: spec.binary,
        api_key: spec.api_key,
    })
    .await?;

    let findings = crate::json::structured(&outcome.result)?;
    Ok(SweepResult {
        findings,
        usage: outcome.usage,
        session_id: outcome.session_id,
        turns: outcome.num_turns,
    })
}

/// Everything one CLI invocation needs, independent of what it is being asked
/// to do. Shared by sweeps and proof attempts, which differ only in prompt,
/// output schema and tool policy.
pub(crate) struct Run<'a> {
    pub(crate) repo: &'a Path,
    pub(crate) model: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) schema: Value,
    pub(crate) allowed: &'a str,
    pub(crate) denied: &'a str,
    pub(crate) max_turns: u32,
    pub(crate) timeout: Duration,
    pub(crate) binary: Option<&'a str>,
    pub(crate) api_key: Option<&'a str>,
}

pub(crate) async fn invoke(run: Run<'_>) -> Result<ResultEnvelope, ProviderError> {
    let binary = match run.binary {
        Some(path) => PathBuf::from(path),
        None => resolve_binary().ok_or_else(not_found)?,
    };
    let binary = binary.to_string_lossy().into_owned();

    let args = build_args(&run);
    let env: Vec<(String, String)> = run
        .api_key
        .map(|key| vec![("ANTHROPIC_API_KEY".to_string(), key.to_string())])
        .unwrap_or_default();

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
        let message = preview(output.stderr.trim(), 2000);
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
fn build_args(run: &Run<'_>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--print".into(),
        "--safe-mode".into(),
        "--output-format".into(),
        "json".into(),
        "--json-schema".into(),
        run.schema.to_string(),
        "--max-turns".into(),
        run.max_turns.to_string(),
        "--allowedTools".into(),
        run.allowed.into(),
    ];
    if !run.denied.is_empty() {
        args.push("--disallowedTools".into());
        args.push(run.denied.into());
    }
    if !run.model.trim().is_empty() {
        args.push("--model".into());
        args.push(run.model.trim().to_string());
    }
    args
}

/// Response envelope from `--output-format json`.
#[derive(Debug, Deserialize)]
pub(crate) struct ResultEnvelope {
    #[serde(default)]
    pub(crate) result: Value,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run<'a>(model: &'a str) -> Run<'a> {
        Run {
            repo: Path::new("."),
            model,
            prompt: "",
            schema: finding_schema(),
            allowed: READ_ONLY_TOOLS,
            denied: READ_ONLY_DENIED,
            max_turns: 12,
            timeout: Duration::from_secs(60),
            binary: None,
            api_key: None,
        }
    }

    #[test]
    fn read_only_sweeps_cannot_be_granted_write_tools() {
        let args = build_args(&run("sonnet"));
        let index = args.iter().position(|a| a == "--disallowedTools");
        let denied = index.and_then(|i| args.get(i + 1)).map(String::as_str);
        assert_eq!(denied, Some(READ_ONLY_DENIED));
        assert!(!args.iter().any(|a| a.contains("dangerously-skip")));
    }

    #[test]
    fn customizations_are_disabled_so_the_reviewed_repo_cannot_alter_the_review() {
        assert!(
            build_args(&run("sonnet"))
                .iter()
                .any(|a| a == "--safe-mode")
        );
    }

    #[test]
    fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
        assert!(!build_args(&run("   ")).iter().any(|a| a == "--model"));
    }

    #[test]
    fn the_schema_is_passed_as_one_argv_entry_not_shell_text() {
        let args = build_args(&run("sonnet"));
        let index = args.iter().position(|a| a == "--json-schema");
        let schema = index
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .unwrap_or("");
        let parsed: Value = serde_json::from_str(schema).unwrap_or(Value::Null);
        assert_eq!(parsed["type"], "object");
    }

    #[test]
    fn an_empty_denylist_omits_the_flag_rather_than_passing_an_empty_value() {
        let mut spec = run("sonnet");
        spec.denied = "";
        assert!(!build_args(&spec).iter().any(|a| a == "--disallowedTools"));
    }
}
