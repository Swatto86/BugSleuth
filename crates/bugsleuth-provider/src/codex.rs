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

mod apply;
mod discover;
mod recover;
mod scratch;

pub use apply::apply;

use scratch::{event_error, not_found, scratch_dir, write_file};

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

/// What the sandbox is allowed to do.
///
/// A sweep is read-only: the operating system refuses a write, which is a far
/// stronger guarantee than asking the agent not to. Applying fixes genuinely has
/// to write, so it gets `workspace-write` — and is only ever pointed at the
/// user's own checkout, which they were shown and chose to hand over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sandbox {
    ReadOnly,
    WorkspaceWrite,
}

impl Sandbox {
    fn flag(self) -> &'static str {
        match self {
            Sandbox::ReadOnly => "read-only",
            Sandbox::WorkspaceWrite => "workspace-write",
        }
    }
}

/// Run one read-only lane sweep through Codex.
pub async fn sweep(spec: CodexSweep<'_>) -> Result<CodexResult, ProviderError> {
    let (findings, salvaged) = invoke(Invoke {
        dir: spec.repo,
        model: spec.model,
        effort: spec.effort,
        brief: spec.brief,
        timeout: spec.timeout,
        binary: spec.binary,
        schema: finding_schema(),
        sandbox: Sandbox::ReadOnly,
    })
    .await?;
    Ok(CodexResult { findings, salvaged })
}

pub(crate) struct Invoke<'a> {
    pub(crate) dir: &'a Path,
    pub(crate) model: &'a str,
    /// Reasoning effort. Empty means the CLI's own default.
    pub(crate) effort: &'a str,
    pub(crate) brief: &'a str,
    pub(crate) timeout: Duration,
    pub(crate) binary: Option<&'a str>,
    /// The shape the reply must take. `Value::Null` means none is imposed —
    /// applying fixes answers in prose for a person, and a schema would cost a
    /// turn to say less.
    pub(crate) schema: serde_json::Value,
    pub(crate) sandbox: Sandbox,
}

async fn invoke<T: serde::de::DeserializeOwned>(
    spec: Invoke<'_>,
) -> Result<(T, bool), ProviderError> {
    let (answer, salvaged) = invoke_text(spec).await?;
    let value = serde_json::from_str(&answer).unwrap_or(serde_json::Value::String(answer));
    crate::json::structured(&value).map(|value| (value, salvaged))
}

/// One invocation, answering with whatever the CLI's final message was.
pub(crate) async fn invoke_text(spec: Invoke<'_>) -> Result<(String, bool), ProviderError> {
    // The accepted reasoning efforts belong to the model, not the CLI, so an
    // effort forwarded to `model_reasoning_effort` is validated against the
    // model's catalogue before anything is spent. Here rather than in one
    // caller, so structured sweeps and prose apply calls are both covered; an
    // apply passes an empty effort and is unaffected.
    crate::models::validate_effort(VENDOR, spec.model, spec.effort).await?;

    let binary = match spec.binary {
        Some(path) => PathBuf::from(path),
        None => discover::resolve_binary().ok_or_else(not_found)?,
    };

    // Codex takes its schema as a file and can write its final answer to one, so
    // both need a scratch directory. It is created inside the system temp area,
    // never inside the repository under review — a review must not leave litter
    // in the thing it is reviewing.
    let scratch = scratch_dir()?;
    // Cleaned on *every* exit from here on. The manual removal used to sit after
    // the await, so it ran only on a normal return: cancelling a sweep drops
    // this future at its await point and skipped it, and a failed initial write
    // returned through `?` before reaching it — both leaving a
    // `bugsleuth-codex-*` directory in the system temp area for good, one more
    // per cancellation. A drop guard runs on cancel, early `?`, and normal
    // return alike.
    let _scratch = Cleanup(scratch.clone());
    let schema_path = scratch.join("schema.json");
    let answer_path = scratch.join("answer.json");
    if !spec.schema.is_null() {
        write_file(&schema_path, &spec.schema.to_string())?;
    }

    let args = build_args(&spec, &schema_path, &answer_path);
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &args,
        cwd: spec.dir,
        stdin: Some(spec.brief.as_bytes()),
        env: &[],
        timeout: spec.timeout,
        what: "codex CLI",
    })
    .await;

    recover::finish_or_resume(
        output,
        &binary.to_string_lossy(),
        &spec,
        &schema_path,
        &answer_path,
    )
    .await
}

/// Removes a directory tree when dropped, so the Codex scratch area is cleaned
/// on a cancelled future and an early `?` as well as on the normal return. A
/// removal that fails is best-effort — it must not mask the provider's result.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn finish(
    output: Result<crate::process::CliOutput, crate::process::ProcessError>,
    answer_path: &Path,
) -> Result<String, ProviderError> {
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
    // Returned raw. Making sense of it belongs to the caller, because not every
    // caller wants the same thing: a sweep needs a schema-shaped object, and an
    // apply needs the prose. This used to parse here regardless, so applying
    // fixes — whose answer is a paragraph — failed with "the reply contained no
    // JSON object" after the model had already done the work.
    Ok(answer)
}

/// The flags every Codex invocation carries, whatever it is for.
///
/// One list, because a sign-in check that invokes the CLI differently from a
/// sweep is not checking the sweep — it can pass while every real run fails, or
/// fail while runs are fine. The first sign-in probe dropped
/// `--skip-git-repo-check` and reported a working Codex as unusable.
///
/// `--ignore-user-config` and `--ignore-rules` are the security-relevant pair:
/// neither the machine's own configuration nor the reviewed repository's rules
/// may change what the model is told to do. Sessions are persisted so a timed
/// out process can resume its existing work. The set is taken from Eir, which
/// drives the same CLI under the same subscription and arrived at it first.
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

/// Build the non-interactive argv.
///
/// `--ignore-user-config` and `--ignore-rules` are Codex's equivalent of
/// Claude's `--safe-mode`: without them the review would load the reviewed
/// repository's own rules, which is both a security problem and a
/// reproducibility one. `--sandbox read-only` is stronger than a tool
/// allowlist — the operating system refuses the write, not the agent.
fn build_args(spec: &Invoke<'_>, schema: &Path, answer: &Path) -> Vec<String> {
    let mut args: Vec<String> = SHARED_FLAGS.iter().map(|s| (*s).to_string()).collect();
    // Measured, not assumed: with `--ignore-user-config` this CLI refuses every
    // patch as "writing is blocked by read-only sandbox", whatever `--sandbox`
    // says — and `-c sandbox_mode=…` and `-c approval_policy=…` do not restore
    // it either. So an invocation that has to write cannot also ignore the
    // machine's configuration, and the honest choice is to keep the writing.
    //
    // `--ignore-rules` stays either way. That is the flag which keeps the
    // reviewed repository — untrusted input — from supplying its own execution
    // policy, and it does not interfere.
    if spec.sandbox == Sandbox::WorkspaceWrite {
        args.retain(|flag| flag != "--ignore-user-config");
        args.push("--ephemeral".into());
    }
    args.push("--json".into());
    args.push("--sandbox".into());
    args.push(spec.sandbox.flag().into());
    // No schema file is written when none was asked for, so naming one here
    // would point the CLI at a path that does not exist.
    if !spec.schema.is_null() {
        args.push("--output-schema".into());
        args.push(schema.to_string_lossy().into_owned());
    }
    args.push("--output-last-message".into());
    args.push(answer.to_string_lossy().into_owned());

    let model = spec.model.trim();
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.to_string());
    }
    // Codex has no effort flag; it is a config key set for this invocation
    // only. `--ignore-user-config` is already set, so this is the sole source
    // of the value rather than an override of whatever the machine had.
    let effort = spec.effort.trim();
    if !effort.is_empty() {
        args.push("-c".into());
        args.push(format!("model_reasoning_effort=\"{effort}\""));
    }
    // A bare `-` tells Codex to read the prompt from stdin, so a long brief
    // cannot hit the command-line length limit.
    args.push("-".into());
    args
}

/// Where the Codex CLI is, if it is installed.
///
/// Exposed for the model catalogue, which needs the binary without wanting
/// anything else this module does. Mirrors `kilo::binary_path`.
#[must_use]
pub fn binary_path() -> Option<PathBuf> {
    discover::resolve_binary()
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

/// Prove the machine holds a ChatGPT session, by using it.
pub async fn signin() -> crate::signin::SignIn {
    let Some(binary) = discover::resolve_binary() else {
        return crate::signin::SignIn::Failed("the codex CLI could not be found".to_string());
    };
    // The same flags a real sweep uses, and that is the point: a check that
    // invokes the CLI differently from the work is not checking the work. It
    // can pass while every sweep fails, or fail while sweeps are fine.
    //
    // This one did the latter. The first version dropped `--skip-git-repo-check`,
    // so Codex refused with "Not inside a trusted directory" — the app runs
    // from wherever it was started, not a repository — and a perfectly good
    // session was reported as unusable, with a message pointing at the wrong
    // problem entirely. `--ignore-user-config` and `--ignore-rules` matter for
    // the same reason they matter in a sweep: neither the machine's own
    // configuration nor a repository's rules should change the answer.
    let args: Vec<String> = SHARED_FLAGS
        .iter()
        .chain(["--sandbox", "read-only", "-"].iter())
        .map(|a| (*a).to_string())
        .collect();
    crate::signin::one_shot(&binary.to_string_lossy(), &args, "codex", str::to_string).await
}

#[cfg(test)]
#[path = "codex/tests.rs"]
mod tests;
