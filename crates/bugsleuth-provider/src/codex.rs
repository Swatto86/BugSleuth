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

use bugsleuth_domain::{RawFindings, finding_schema};

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

mod discover;

pub(crate) const VENDOR: &str = "codex";

pub struct CodexSweep<'a> {
    pub repo: &'a Path,
    /// Model id, e.g. `gpt-5.6-codex`. Empty means the CLI's own default.
    pub model: &'a str,
    pub brief: &'a str,
    pub timeout: Duration,
    pub binary: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct CodexResult {
    pub findings: RawFindings,
}

/// Run one read-only lane sweep through Codex.
pub async fn sweep(spec: CodexSweep<'_>) -> Result<CodexResult, ProviderError> {
    let binary = match spec.binary {
        Some(path) => PathBuf::from(path),
        None => discover::resolve_binary().ok_or_else(not_found)?,
    };

    // Codex takes its schema as a file and can write its final answer to one, so
    // both need a scratch directory. It is created inside the system temp area,
    // never inside the repository under review — a review must not leave litter
    // in the thing it is reviewing.
    let scratch = scratch_dir()?;
    let schema_path = scratch.join("schema.json");
    let answer_path = scratch.join("answer.json");
    write_file(&schema_path, &finding_schema().to_string())?;

    let args = build_args(&spec, &schema_path, &answer_path);
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &args,
        cwd: spec.repo,
        stdin: Some(spec.brief.as_bytes()),
        env: &[],
        timeout: spec.timeout,
        what: "codex CLI",
    })
    .await;

    let result = finish(output, &answer_path);
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn finish(
    output: Result<crate::process::CliOutput, crate::process::ProcessError>,
    answer_path: &Path,
) -> Result<CodexResult, ProviderError> {
    let output = output?;

    if !output.succeeded() {
        let code = output.code.unwrap_or(-1);
        // Codex reports failures as events on stdout as well as on stderr, and
        // the event usually carries the more useful message.
        let message = event_error(&output.stdout)
            .unwrap_or_else(|| preview(output.stderr.trim(), 2000))
            .trim()
            .to_string();
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

    let answer = std::fs::read_to_string(answer_path).map_err(|e| ProviderError::Envelope {
        vendor: VENDOR,
        detail: format!("the CLI wrote no final answer: {e}"),
    })?;
    if answer.trim().is_empty() {
        return Err(ProviderError::Empty(VENDOR));
    }

    let value = serde_json::from_str(&answer).unwrap_or(serde_json::Value::String(answer));
    Ok(CodexResult {
        findings: crate::json::structured(&value)?,
    })
}

/// Build the non-interactive argv.
///
/// `--ignore-user-config` and `--ignore-rules` are Codex's equivalent of
/// Claude's `--safe-mode`: without them the review would load the reviewed
/// repository's own rules, which is both a security problem and a
/// reproducibility one. `--sandbox read-only` is stronger than a tool
/// allowlist — the operating system refuses the write, not the agent.
fn build_args(spec: &CodexSweep<'_>, schema: &Path, answer: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["--ask-for-approval", "never", "exec", "--json"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    args.extend(
        [
            "--sandbox",
            "read-only",
            "--skip-git-repo-check",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--color",
            "never",
        ]
        .iter()
        .map(|s| (*s).to_string()),
    );
    args.push("--output-schema".into());
    args.push(schema.to_string_lossy().into_owned());
    args.push("--output-last-message".into());
    args.push(answer.to_string_lossy().into_owned());

    let model = spec.model.trim();
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.to_string());
    }
    // A bare `-` tells Codex to read the prompt from stdin, so a long brief
    // cannot hit the command-line length limit.
    args.push("-".into());
    args
}

/// First error carried by a Codex event, if any.
fn event_error(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let event: serde_json::Value = serde_json::from_str(line).ok()?;
        match event["type"].as_str()? {
            "turn.failed" => event["error"]["message"].as_str().map(|s| preview(s, 2000)),
            "error" => event["message"].as_str().map(|s| preview(s, 2000)),
            _ => None,
        }
    })
}

fn scratch_dir() -> Result<PathBuf, ProviderError> {
    let dir = std::env::temp_dir().join(format!("bugsleuth-codex-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| ProviderError::Scratch {
        vendor: VENDOR,
        detail: e.to_string(),
    })?;
    Ok(dir)
}

fn write_file(path: &Path, contents: &str) -> Result<(), ProviderError> {
    std::fs::write(path, contents).map_err(|e| ProviderError::Scratch {
        vendor: VENDOR,
        detail: format!("{}: {e}", path.display()),
    })
}

fn not_found() -> ProviderError {
    ProviderError::NotFound {
        vendor: VENDOR,
        hint: "Install the Codex CLI and sign in with `codex login`, or pass an explicit binary \
               path."
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec<'a>(model: &'a str) -> CodexSweep<'a> {
        CodexSweep {
            repo: Path::new("."),
            model,
            brief: "",
            timeout: Duration::from_secs(60),
            binary: None,
        }
    }

    #[test]
    fn a_sweep_runs_read_only_and_ignores_the_reviewed_repos_own_config() {
        let args = build_args(
            &spec("gpt-5.6-codex"),
            Path::new("s.json"),
            Path::new("a.json"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("--sandbox read-only"));
        assert!(joined.contains("--ignore-user-config"));
        assert!(joined.contains("--ignore-rules"));
        assert!(!joined.contains("dangerously"));
    }

    #[test]
    fn the_prompt_comes_from_stdin_so_a_long_brief_cannot_overflow_the_command_line() {
        let args = build_args(&spec(""), Path::new("s.json"), Path::new("a.json"));
        assert_eq!(args.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
        let args = build_args(&spec("  "), Path::new("s.json"), Path::new("a.json"));
        assert!(!args.iter().any(|a| a == "-m"));
    }

    #[test]
    fn the_schema_and_answer_paths_are_passed_as_files_not_inline_json() {
        let args = build_args(&spec(""), Path::new("s.json"), Path::new("a.json"));
        let after = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
        };
        assert_eq!(after("--output-schema"), Some("s.json"));
        assert_eq!(after("--output-last-message"), Some("a.json"));
    }

    #[test]
    fn a_failure_event_on_stdout_is_preferred_over_empty_stderr() {
        let stdout = r#"{"type":"turn.failed","error":{"message":"rate limited"}}"#;
        assert_eq!(event_error(stdout).as_deref(), Some("rate limited"));
        assert_eq!(event_error("not json at all"), None);
    }
}
