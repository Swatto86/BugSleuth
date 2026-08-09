//! Resume a read-only Codex sweep whose process timed out.

use std::path::Path;
use std::time::Duration;

use super::{Invoke, Sandbox, VENDOR, build_args, finish};
use crate::error::ProviderError;
use crate::process::{self, CliOutput, Invocation, ProcessError};

const ASK: &str = "The previous CLI run timed out. Return only the final answer from work already completed in this conversation. Do not inspect or change anything else.";

pub(super) async fn finish_or_resume(
    output: Result<CliOutput, ProcessError>,
    binary: &str,
    spec: &Invoke<'_>,
    schema: &Path,
    answer: &Path,
) -> Result<(String, bool), ProviderError> {
    let error = match output {
        Err(error @ ProcessError::Timeout { .. }) if spec.sandbox == Sandbox::ReadOnly => error,
        other => return finish(other, answer).map(|answer| (answer, false)),
    };
    let Some(session) = error.output().and_then(|out| thread_id(&out.stdout)) else {
        return Err(error.into());
    };
    let original = error.to_string();
    if let Err(error) = std::fs::remove_file(answer)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(ProviderError::Recovery {
            vendor: VENDOR,
            original,
            recovery: format!("could not clear the interrupted answer: {error}"),
        });
    }
    let args = resume_args(spec, schema, answer, &session);
    let recovery = process::run(Invocation {
        binary,
        args: &args,
        cwd: spec.dir,
        stdin: Some(ASK.as_bytes()),
        env: &[],
        timeout: spec.timeout.min(Duration::from_secs(300)),
        what: "codex CLI recovery",
    })
    .await;
    finish(recovery, answer)
        .map(|answer| (answer, true))
        .map_err(|recovery| ProviderError::Recovery {
            vendor: VENDOR,
            original,
            recovery: recovery.to_string(),
        })
}

fn thread_id(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let event: serde_json::Value = serde_json::from_str(line).ok()?;
        (event["type"] == "thread.started")
            .then(|| event["thread_id"].as_str().map(str::to_string))?
            .filter(|id| !id.trim().is_empty())
    })
}

fn resume_args(spec: &Invoke<'_>, schema: &Path, answer: &Path, session: &str) -> Vec<String> {
    let mut args = build_args(spec, schema, answer);
    let prompt = args.pop().unwrap_or_else(|| "-".to_string());
    args.extend(["resume".to_string(), session.to_string(), prompt]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_started_thread_is_the_only_event_worth_resuming() {
        let stdout = "not json\n{\"type\":\"item.completed\"}\n{\"type\":\"thread.started\",\"thread_id\":\"abc\"}";
        assert_eq!(thread_id(stdout).as_deref(), Some("abc"));
        assert_eq!(
            thread_id(r#"{"type":"thread.started","thread_id":""}"#),
            None
        );
    }
}
