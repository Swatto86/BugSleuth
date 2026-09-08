use super::*;

#[tokio::test]
async fn repository_review_launches_codex() {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-enabled-codex-review-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture repository");

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
    #[cfg(windows)]
    std::fs::write(&stub, script).expect("write fake Codex CLI");
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
    let _ = std::fs::remove_dir_all(&dir);

    assert!(launched, "the review never launched Codex: {outcome:?}");
    assert!(
        outcome.is_ok(),
        "the review did not return findings: {outcome:?}"
    );
}
