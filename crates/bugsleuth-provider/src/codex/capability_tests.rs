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
        "@echo off\r\n>launched.txt echo launched\r\n:args\r\nif \"%~1\"==\"\" exit /b 2\r\nif \"%~1\"==\"--output-last-message\" goto answer\r\nshift\r\ngoto args\r\n:answer\r\n>\"%~2\" echo {\"findings\":[]}\r\nexit /b 0\r\n",
    );
    #[cfg(unix)]
    let (stub, script) = (
        dir.join("codex"),
        "#!/bin/sh\nprintf launched > launched.txt\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--output-last-message\" ]; then\n    printf '%s\\n' '{\"findings\":[]}' > \"$2\"\n    exit 0\n  fi\n  shift\ndone\nexit 2\n",
    );
    std::fs::write(&stub, script).expect("write fake Codex CLI");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("make fake CLI executable");
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
