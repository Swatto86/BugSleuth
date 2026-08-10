//! Refusing to hand a fix prompt to Kilo in the real repository.
//!
//! Kilo has no per-invocation write confinement. Its default agent can be
//! replaced by repository configuration, so automatically approving that agent
//! would let the repository choose which host capabilities receive write-task
//! authority. Sweeps can use a disposable worktree; an apply cannot.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;

use super::VENDOR;

/// Apply the fixes described in `prompt`, returning the model's own account.
pub async fn apply(
    _repo: &Path,
    _model: &str,
    _effort: &str,
    _prompt: &str,
    _timeout: Duration,
) -> Result<String, ProviderError> {
    Err(ProviderError::CapabilityUnavailable {
        vendor: VENDOR,
        capability: "apply",
        reason: "its default agent can be replaced by repository configuration, so BugSleuth cannot safely grant it write access. Apply the generated handoff manually in an isolated environment."
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("bugsleuth-kilo-apply-tests")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch directory");
        dir
    }

    fn install_fake_cli(home: &Path) {
        let binary = if cfg!(windows) {
            home.join("AppData/Roaming/npm/kilo.cmd")
        } else {
            home.join(".local/bin/kilo")
        };
        std::fs::create_dir_all(binary.parent().expect("fake CLI has a parent"))
            .expect("create fake CLI directory");
        let script = if cfg!(windows) {
            "@echo off\r\necho invoked>invoked.txt\r\nexit /b 9\r\n"
        } else {
            "#!/bin/sh\nprintf invoked > invoked.txt\nexit 9\n"
        };
        std::fs::write(&binary, script).expect("write fake CLI");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut permissions = std::fs::metadata(&binary)
                .expect("read fake CLI permissions")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&binary, permissions).expect("make fake CLI executable");
        }
    }

    #[tokio::test]
    async fn apply_is_disabled_before_the_kilo_cli_can_launch() {
        const ROLE: &str = "BUGSLEUTH_KILO_APPLY_TEST_ROLE";
        const REPO: &str = "BUGSLEUTH_KILO_APPLY_TEST_REPO";
        const TEST: &str = "kilo::apply::tests::apply_is_disabled_before_the_kilo_cli_can_launch";

        if std::env::var(ROLE).as_deref() != Ok("child") {
            let repo = scratch();
            install_fake_cli(&repo);
            let status = tokio::process::Command::new(
                std::env::current_exe().expect("find provider test executable"),
            )
            .args(["--exact", TEST, "--nocapture"])
            .env(ROLE, "child")
            .env(REPO, &repo)
            .env("HOME", &repo)
            .env("USERPROFILE", &repo)
            .status()
            .await
            .expect("run isolated apply test");
            assert!(status.success(), "isolated apply test failed");
            let _ = std::fs::remove_dir_all(repo);
            return;
        }

        let repo = std::path::PathBuf::from(std::env::var_os(REPO).expect("test repo is set"));
        let outcome = apply(
            &repo,
            "kilo/openai/gpt-5.6-sol",
            "high",
            "apply fixes",
            Duration::from_secs(10),
        )
        .await;
        assert!(
            !repo.join("invoked.txt").exists(),
            "the untrusted Kilo apply CLI was launched: {outcome:?}"
        );
        let message = outcome
            .expect_err("Kilo apply must fail closed")
            .to_string();
        assert!(
            message.contains("kilo apply is unavailable")
                && message
                    .contains("Apply the generated handoff manually in an isolated environment"),
            "the refusal was not actionable: {message}"
        );
    }
}
