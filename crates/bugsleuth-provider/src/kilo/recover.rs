//! Resume a Kilo sweep whose process timed out.

use std::path::Path;
use std::time::Duration;

use super::{KiloSweep, VENDOR, assistant_text_or_error, build_args, events};
use crate::error::ProviderError;
use crate::process::{self, Invocation, ProcessError};

const ASK: &str = "The previous CLI run timed out. Return only the final JSON answer from work already completed in this conversation. Do not inspect any files, continue the review, or invent findings.";

pub(super) async fn timeout(
    error: ProcessError,
    spec: &KiloSweep<'_>,
    binary: &Path,
) -> Result<String, ProviderError> {
    let Some(session) = error
        .output()
        .and_then(|out| events::session_id(&out.stdout))
    else {
        return Err(error.into());
    };
    let original = error.to_string();
    let recovery: Result<String, ProviderError> = async {
        let args = recovery_args(spec, &session);
        let env = super::sweep_environment();
        let output = process::run(Invocation {
            binary: &binary.to_string_lossy(),
            args: &args,
            cwd: spec.worktree,
            stdin: Some(ASK.as_bytes()),
            env: &env,
            timeout: spec.timeout.min(Duration::from_secs(300)),
            what: "kilo CLI recovery",
        })
        .await?;
        assistant_text_or_error(&output)
    }
    .await;
    recovery.map_err(|recovery| ProviderError::Recovery {
        vendor: VENDOR,
        original,
        recovery: recovery.to_string(),
    })
}

fn recovery_args(spec: &KiloSweep<'_>, session: &str) -> Vec<String> {
    let mut args = build_args(spec);
    args.extend(["--session".to_string(), session.to_string()]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_keeps_the_sweep_guards_and_the_throwaway_worktree() {
        let spec = KiloSweep {
            worktree: Path::new("reviewed"),
            model: "kilo/model",
            effort: "high",
            brief: "",
            timeout: Duration::from_secs(60),
            binary: None,
        };
        let args = recovery_args(&spec, "abc").join(" ");
        assert!(args.contains("--pure") && args.contains("--agent ask"));
        assert!(args.contains("--dir reviewed") && args.contains("--session abc"));
    }
}
