//! Codex apply is disabled until it can be contained on every supported host.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;

const DISABLED_REASON: &str = "BugSleuth cannot prove that Codex's write-capable sandbox confines host reads and side effects to this repository; apply the generated handoff manually in an isolated environment";

/// Refuse write-capable Codex work before discovering or starting the CLI.
pub async fn apply(
    _repo: &Path,
    _model: &str,
    _effort: &str,
    _prompt: &str,
    _timeout: Duration,
) -> Result<String, ProviderError> {
    Err(ProviderError::CapabilityUnavailable {
        vendor: super::VENDOR,
        capability: "apply",
        reason: DISABLED_REASON.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD_RESULT: &str = "BUGSLEUTH_CODEX_APPLY_CHILD_RESULT";

    #[tokio::test]
    async fn apply_fails_closed_before_launching_codex() {
        if let Some(result_path) = std::env::var_os(CHILD_RESULT) {
            let repo = std::env::current_dir().expect("child repository");
            let outcome = apply(
                &repo,
                "",
                "",
                "untrusted model-produced fix",
                Duration::from_secs(10),
            )
            .await;
            let result = match outcome {
                Ok(text) => format!("ok:{text}"),
                Err(error) => format!("error:{error}"),
            };
            std::fs::write(result_path, result).expect("record child result");
            return;
        }

        let dir = std::env::temp_dir().join(format!(
            "bugsleuth-disabled-codex-apply-{}",
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
            bin.join("codex.cmd"),
            "@echo off\r\n>launched.txt echo launched\r\nexit /b 9\r\n",
        );
        #[cfg(unix)]
        let (stub, script) = {
            let native_bin = home.join(".local/bin");
            std::fs::create_dir_all(&native_bin).expect("create native CLI directory");
            (
                native_bin.join("codex"),
                "#!/bin/sh\nprintf launched > launched.txt\nexit 9\n",
            )
        };
        std::fs::write(&stub, script).expect("write fake Codex CLI");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .expect("make fake CLI executable");
        }
        assert!(stub.is_file(), "fake Codex CLI was not created");

        let result_path = dir.join("result.txt");
        let child = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "codex::apply::tests::apply_fails_closed_before_launching_codex",
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
        assert!(!launched, "the disabled provider still launched Codex");
        assert!(
            result.contains("error:codex apply is unavailable"),
            "{result}"
        );
        assert!(
            result.contains("confines host reads and side effects"),
            "{result}"
        );
    }
}
