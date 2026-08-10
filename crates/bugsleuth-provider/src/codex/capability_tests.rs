use super::*;

#[tokio::test]
async fn repository_review_fails_closed_before_launching_codex() {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-disabled-codex-review-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture repository");

    #[cfg(windows)]
    let (stub, script) = (
        dir.join("codex.cmd"),
        "@echo off\r\n>launched.txt echo launched\r\nexit /b 9\r\n",
    );
    #[cfg(unix)]
    let (stub, script) = (
        dir.join("codex"),
        "#!/bin/sh\nprintf launched > launched.txt\nexit 9\n",
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

    assert!(!launched, "the disabled review still launched Codex");
    assert!(
        matches!(
            outcome,
            Err(ProviderError::CapabilityUnavailable {
                vendor: "codex",
                capability: "repository review",
                ..
            })
        ),
        "the review did not return the capability refusal: {outcome:?}"
    );
}
