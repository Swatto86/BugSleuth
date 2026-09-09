//! One read-only Codex repository review.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::finding_schema;

use crate::error::ProviderError;
use crate::process::{self, Invocation};

use super::scratch::{Cleanup, scratch_dir, write_file};
use super::{CodexResult, CodexSweep, VENDOR, not_found, read_only_args};

#[path = "recover.rs"]
mod recover;

pub(super) struct Invoke<'a> {
    pub(super) dir: &'a Path,
    pub(super) model: &'a str,
    pub(super) effort: &'a str,
    pub(super) brief: &'a str,
    pub(super) timeout: Duration,
    pub(super) binary: Option<&'a str>,
    pub(super) schema: serde_json::Value,
}

/// Run one read-only lane sweep through Codex.
pub async fn sweep(spec: CodexSweep<'_>) -> Result<CodexResult, ProviderError> {
    let brief = format!(
        "{}\n\nSet review_error to an empty string only after inspecting the source and completing the requested review. If repository access, tools, or another blocker prevents the review, put the reason in review_error, even if findings is empty. Do not represent an unperformed review as clean.",
        spec.brief
    );
    let (review, salvaged): (Review, bool) = invoke(Invoke {
        dir: spec.repo,
        model: spec.model,
        effort: spec.effort,
        brief: &brief,
        timeout: spec.timeout,
        binary: spec.binary,
        schema: review_schema(),
    })
    .await?;
    if !review.review_error.trim().is_empty() {
        return Err(ProviderError::CapabilityUnavailable {
            vendor: VENDOR,
            capability: "repository review",
            reason: review.review_error,
        });
    }
    Ok(CodexResult {
        findings: review.findings,
        salvaged,
    })
}

#[derive(serde::Deserialize)]
struct Review {
    review_error: String,
    #[serde(flatten)]
    findings: bugsleuth_domain::RawFindings,
}

/// Provider wire contract, also used to invalidate pre-completion-check caches.
pub fn review_schema() -> serde_json::Value {
    let mut schema = finding_schema();
    schema["required"] = serde_json::json!(["findings", "review_error"]);
    schema["properties"]["review_error"] = serde_json::json!({
        "type": "string",
        "description": "Empty only when the requested source review completed. Otherwise explain what prevented review, including blocked repository reads or unavailable tools."
    });
    schema
}

async fn invoke<T: serde::de::DeserializeOwned>(
    spec: Invoke<'_>,
) -> Result<(T, bool), ProviderError> {
    let (answer, salvaged) = invoke_text(spec).await?;
    let value = serde_json::from_str(&answer).unwrap_or(serde_json::Value::String(answer));
    crate::json::structured(&value).map(|value| (value, salvaged))
}

pub(super) async fn invoke_text(spec: Invoke<'_>) -> Result<(String, bool), ProviderError> {
    crate::models::validate_effort(VENDOR, spec.model, spec.effort).await?;

    let binary = match spec.binary {
        Some(path) => PathBuf::from(path),
        None => super::discover::resolve_binary().ok_or_else(not_found)?,
    };

    let scratch = scratch_dir()?;
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

/// Build the non-interactive argv for a read-only review.
///
/// `--ignore-user-config` and `--ignore-rules` are Codex's equivalent of
/// Claude's `--safe-mode`: without them the review would load the reviewed
/// repository's own rules. `--sandbox read-only` is the write boundary — the
/// operating system refuses the write, not the agent.
pub(super) fn build_args(spec: &Invoke<'_>, schema: &Path, answer: &Path) -> Vec<String> {
    let mut args = read_only_args();
    args.push("--json".into());
    args.push("--sandbox".into());
    args.push("read-only".into());
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
    let effort = spec.effort.trim();
    if !effort.is_empty() {
        args.push("-c".into());
        args.push(format!("model_reasoning_effort=\"{effort}\""));
    }
    args.push("-".into());
    args
}

#[cfg(test)]
#[path = "sweep_tests.rs"]
mod tests;
