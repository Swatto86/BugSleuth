//! Claude Code CLI adapter.
//!
//! Runs `claude --print` non-interactively against a repository and returns the
//! raw findings it reports. Nothing here trusts the model: the result is
//! `RawFindings`, which cannot reach a report without going through anchor
//! verification first.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::{Lane, RawFindings, finding_schema};
use serde::Deserialize;
use serde_json::Value;

use crate::process::{self, Invocation, ProcessError, preview};

mod envelope;

pub use envelope::Usage;

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error(
        "the claude CLI could not be found. Install it (`npm install -g @anthropic-ai/claude-code`) and sign in with `claude`, or set an explicit binary path."
    )]
    NotFound,
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("claude CLI exited with code {code}: {message}")]
    Failed { code: i32, message: String },
    #[error(
        "claude CLI exited with code {code} and produced no diagnostic output — usually a transient overload or rate limit"
    )]
    FailedSilently { code: i32 },
    #[error("claude CLI produced no output")]
    Empty,
    #[error("could not read the claude CLI's response envelope: {0}")]
    Envelope(String),
    #[error("the model's reply was not valid findings JSON: {0}")]
    Schema(String),
}

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
pub async fn sweep(spec: ClaudeSweep<'_>) -> Result<SweepResult, ClaudeError> {
    let binary = match spec.binary {
        Some(path) => PathBuf::from(path),
        None => resolve_binary().ok_or(ClaudeError::NotFound)?,
    };
    let binary = binary.to_string_lossy().into_owned();

    let args = build_args(&spec);
    let env: Vec<(String, String)> = spec
        .api_key
        .map(|key| vec![("ANTHROPIC_API_KEY".to_string(), key.to_string())])
        .unwrap_or_default();

    let output = process::run(Invocation {
        binary: &binary,
        args: &args,
        cwd: spec.repo,
        stdin: Some(spec.brief.as_bytes()),
        env: &env,
        timeout: spec.timeout,
        what: "claude CLI",
    })
    .await?;

    if !output.succeeded() {
        let code = output.code.unwrap_or(-1);
        let message = preview(output.stderr.trim(), 2000);
        return Err(if message.is_empty() {
            ClaudeError::FailedSilently { code }
        } else {
            ClaudeError::Failed { code, message }
        });
    }

    let stdout = output.stdout.trim();
    if stdout.is_empty() {
        return Err(ClaudeError::Empty);
    }

    let envelope = envelope::parse(stdout)?;
    let findings = envelope::findings_from_result(&envelope.result)?;
    Ok(SweepResult {
        findings,
        usage: envelope.usage,
        session_id: envelope.session_id,
        turns: envelope.num_turns,
    })
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
pub async fn probe() -> Result<String, ClaudeError> {
    let binary = resolve_binary().ok_or(ClaudeError::NotFound)?;
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
        return Err(ClaudeError::Failed {
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 500),
        });
    }
    Ok(output.stdout.trim().to_string())
}

/// Build the non-interactive argv.
///
/// Two choices worth spelling out:
///
/// `--safe-mode` disables every customization the machine or the repository
/// under review would otherwise inject — CLAUDE.md, hooks, skills, MCP servers,
/// custom agents. Without it, reviewing a repository would execute that
/// repository's hooks, and the review's behaviour would silently depend on
/// whatever is in the developer's global config. Authentication is unaffected,
/// so the signed-in subscription session still applies.
///
/// `--allowedTools` is an explicit allowlist rather than
/// `--dangerously-skip-permissions`. A read-only sweep genuinely cannot write.
fn build_args(spec: &ClaudeSweep<'_>) -> Vec<String> {
    let schema = finding_schema().to_string();
    let mut args: Vec<String> = vec![
        "--print".into(),
        "--safe-mode".into(),
        "--output-format".into(),
        "json".into(),
        "--json-schema".into(),
        schema,
        "--max-turns".into(),
        spec.max_turns.to_string(),
        "--allowedTools".into(),
        "Read,Glob,Grep".into(),
        "--disallowedTools".into(),
        "Edit,Write,NotebookEdit,Bash,WebFetch,WebSearch".into(),
    ];
    if !spec.model.trim().is_empty() {
        args.push("--model".into());
        args.push(spec.model.trim().to_string());
    }
    let _ = spec.lane;
    args
}

/// Locate the CLI, preferring a real executable over an npm shim.
///
/// On Windows the npm `claude.cmd` shim has to be run through `cmd.exe`, which
/// would re-expose every argument to shell parsing — and one of our arguments is
/// a JSON Schema full of quotes and braces. The native `claude.exe` next to it
/// takes argv as an array with no shell in the path at all.
fn resolve_binary() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);

    if let Some(home) = home {
        let candidates = [
            home.join(".local/bin/claude.exe"),
            home.join("AppData/Roaming/npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe"),
            home.join(".local/bin/claude"),
        ];
        for candidate in candidates {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    which("claude")
}

/// Minimal PATH lookup. A dependency for this would be three lines of value.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions: &[&str] = if cfg!(windows) {
        &["exe", "cmd"]
    } else {
        &[""]
    };
    for directory in std::env::split_paths(&path) {
        for extension in extensions {
            let candidate = if extension.is_empty() {
                directory.join(name)
            } else {
                directory.join(format!("{name}.{extension}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
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

    fn spec<'a>(model: &'a str) -> ClaudeSweep<'a> {
        ClaudeSweep {
            repo: Path::new("."),
            lane: Lane::Correctness,
            model,
            brief: "",
            timeout: Duration::from_secs(60),
            max_turns: 12,
            binary: None,
            api_key: None,
        }
    }

    #[test]
    fn read_only_sweeps_cannot_be_granted_write_tools() {
        let args = build_args(&spec("sonnet"));
        let joined = args.join(" ");
        assert!(joined.contains("--disallowedTools"));
        let index = args.iter().position(|a| a == "--disallowedTools");
        let denied = index.and_then(|i| args.get(i + 1)).map(String::as_str);
        assert_eq!(
            denied,
            Some("Edit,Write,NotebookEdit,Bash,WebFetch,WebSearch")
        );
        assert!(!joined.contains("--dangerously-skip-permissions"));
    }

    #[test]
    fn customizations_are_disabled_so_the_reviewed_repo_cannot_alter_the_review() {
        let args = build_args(&spec("sonnet"));
        assert!(args.iter().any(|a| a == "--safe-mode"));
    }

    #[test]
    fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
        let args = build_args(&spec("   "));
        assert!(!args.iter().any(|a| a == "--model"));
    }

    #[test]
    fn the_schema_is_passed_as_one_argv_entry_not_shell_text() {
        let args = build_args(&spec("sonnet"));
        let index = args.iter().position(|a| a == "--json-schema");
        let schema = index
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .unwrap_or("");
        let parsed: Value = serde_json::from_str(schema).unwrap_or(Value::Null);
        assert_eq!(parsed["type"], "object");
    }
}
