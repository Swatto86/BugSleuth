use super::*;

async fn run_review(answer: &str) -> Result<CodexResult, ProviderError> {
    let dir = scratch::scratch_dir().unwrap();
    let _cleanup = scratch::Cleanup(dir.clone());

    #[cfg(windows)]
    let (stub, script) = (
        dir.join("codex.cmd"),
        "@echo off\r\nfindstr /R \".*\" > nul\r\n>launched.txt echo launched\r\n:args\r\nif \"%~1\"==\"\" exit /b 2\r\nif \"%~1\"==\"--output-last-message\" goto answer\r\nshift\r\ngoto args\r\n:answer\r\n>\"%~2\" echo {\"findings\":[]}\r\nexit /b 0\r\n",
    );
    #[cfg(unix)]
    let (stub, script) = (
        dir.join("codex"),
        "#!/bin/sh\ncat >/dev/null\nprintf launched > launched.txt\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--output-last-message\" ]; then\n    printf '%s\\n' '{\"findings\":[]}' > \"$2\"\n    exit 0\n  fi\n  shift\ndone\nexit 2\n",
    );
    let script = script.replace(r#"{"findings":[]}"#, answer);
    #[cfg(windows)]
    std::fs::write(&stub, &script).expect("write fake Codex CLI");
    #[cfg(unix)]
    {
        // Write in a separate process: another parallel test may fork while a
        // parent-side file descriptor is writable. Until that child execs it
        // can keep our executable busy, making this test fail with ETXTBSY.
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut writer = Command::new("sh")
            .args([
                "-c",
                "umask 077; cat > \"$1\" && chmod 700 \"$1\"",
                "fixture",
            ])
            .arg(&stub)
            .stdin(Stdio::piped())
            .spawn()
            .expect("start fixture writer");
        writer
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .expect("write fixture script");
        assert!(writer.wait().expect("wait for fixture writer").success());
    }

    let binary = stub.to_string_lossy().into_owned();
    let outcome = sweep(CodexSweep {
        repo: &dir,
        model: "",
        effort: "",
        brief: "untrusted repository review",
        timeout: Duration::from_secs(10),
        binary: Some(&binary),
    })
    .await;
    let launched = dir.join("launched.txt").exists();
    assert!(launched, "the review never launched Codex: {outcome:?}");
    outcome
}

#[tokio::test]
async fn repository_review_launches_codex() {
    let outcome = run_review(r#"{"findings":[],"review_error":""}"#).await;
    assert!(
        outcome.is_ok(),
        "the review did not return findings: {outcome:?}"
    );
}

#[tokio::test]
async fn a_blocked_review_is_not_an_empty_success() {
    let outcome =
        run_review(r#"{"findings":[],"review_error":"Repository reads blocked by policy"}"#).await;
    let error = outcome.expect_err("blocked review was accepted as clean");
    assert!(
        error
            .to_string()
            .contains("Repository reads blocked by policy")
    );
    assert!(!error.is_transient());
}

#[tokio::test]
async fn a_review_without_completion_status_is_rejected() {
    assert!(run_review(r#"{"findings":[]}"#).await.is_err());
}
