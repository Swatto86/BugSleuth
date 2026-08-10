//! Claude apply is disabled until it can be contained on every supported host.

use std::path::Path;
use std::time::Duration;

use super::ProviderError;

const DISABLED_REASON: &str = "BugSleuth cannot prove that Claude's write and shell tools are confined to this repository with network access blocked; apply the generated handoff manually in an isolated environment";

pub struct ApplyRequest<'a> {
    /// The repository the generated handoff refers to.
    pub repo: &'a Path,
    pub model: &'a str,
    pub effort: &'a str,
    /// The handoff prompt, exactly as it was written to disk.
    pub prompt: &'a str,
    pub timeout: Duration,
    pub max_turns: u32,
}

/// Refuse the write-capable operation before discovering or starting Claude.
pub async fn apply(_request: ApplyRequest<'_>) -> Result<String, ProviderError> {
    Err(ProviderError::CapabilityUnavailable {
        vendor: "claude",
        capability: "apply",
        reason: DISABLED_REASON.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD_RESULT: &str = "BUGSLEUTH_CLAUDE_APPLY_CHILD_RESULT";

    #[tokio::test]
    async fn apply_fails_closed_before_launching_claude() {
        if let Some(result_path) = std::env::var_os(CHILD_RESULT) {
            let repo = std::env::current_dir().expect("child repository");
            let outcome = apply(ApplyRequest {
                repo: &repo,
                model: "",
                effort: "",
                prompt: "untrusted model-produced fix",
                timeout: Duration::from_secs(10),
                max_turns: 1,
            })
            .await;
            let result = match outcome {
                Ok(text) => format!("ok:{text}"),
                Err(error) => format!("error:{error}"),
            };
            std::fs::write(result_path, result).expect("record child result");
            return;
        }

        let dir = std::env::temp_dir().join(format!(
            "bugsleuth-disabled-claude-apply-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        let home = dir.join("home");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&repo).expect("create fixture repository");
        std::fs::create_dir_all(&home).expect("create fixture home");
        std::fs::create_dir_all(&bin).expect("create fixture binary directory");

        #[cfg(windows)]
        let (stub, script) = (
            bin.join("claude.cmd"),
            "@echo off\r\n\
             echo launched > launched.txt\r\n\
             echo {\"result\":\"ran\",\"is_error\":false,\"session_id\":\"known-session\"}\r\n",
        );
        #[cfg(unix)]
        let (stub, script) = {
            let native_bin = home.join(".local/bin");
            std::fs::create_dir_all(&native_bin).expect("create native CLI directory");
            (
                native_bin.join("claude"),
                "#!/bin/sh\n\
             printf launched > launched.txt\n\
                 printf '%s\\n' '{\"result\":\"ran\",\"is_error\":false,\"session_id\":\"known-session\"}'\n",
            )
        };
        std::fs::write(&stub, script).expect("write CLI stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&stub)
                .expect("read stub permissions")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&stub, permissions).expect("make CLI stub executable");
        }
        assert!(stub.is_file(), "fake Claude CLI was not created");

        let result_path = dir.join("result.txt");
        let child = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "claude::apply::tests::apply_fails_closed_before_launching_claude",
                "--nocapture",
            ])
            .current_dir(&repo)
            .env(CHILD_RESULT, &result_path)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("PATH", &bin)
            .output()
            .expect("run isolated apply attempt");
        let launched = repo.join("launched.txt").exists();
        let result = std::fs::read_to_string(&result_path).expect("child apply result");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(child.status.success(), "child failed: {child:?}");
        assert!(!launched, "the disabled provider still launched Claude");
        assert!(
            result.contains("error:claude apply is unavailable"),
            "{result}"
        );
        assert!(result.contains("confined to this repository"), "{result}");
    }
}
