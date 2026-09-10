use super::*;

#[test]
fn local_model_tags_and_variants_are_forwarded_verbatim() {
    let spec = OpenCodeSweep {
        worktree: Path::new("/tmp/review space"),
        model: "ollama/qwen3:8b",
        effort: "thinking",
        brief: "review",
        timeout: Duration::from_secs(1),
        binary: None,
    };
    let args = build_args(&spec, "private-agent");
    assert!(args.windows(2).any(|v| v == ["--model", "ollama/qwen3:8b"]));
    assert!(args.windows(2).any(|v| v == ["--dir", "/tmp/review space"]));
    assert!(args.windows(2).any(|v| v == ["--variant", "thinking"]));
    assert!(!args.iter().any(|v| v == "--auto"));
}

#[test]
fn review_and_apply_have_distinct_permissions() {
    for write in [false, true] {
        let env = environment("private-agent", write);
        let config: serde_json::Value = serde_json::from_str(&env[0].1).unwrap();
        let permission = &config["agent"]["private-agent"]["permission"];
        assert_eq!(permission["*"], "deny");
        assert_eq!(permission["read"], "allow");
        assert_eq!(permission["glob"], "allow");
        assert_eq!(permission["grep"], "allow");
        assert_eq!(permission["edit"].as_str(), write.then_some("allow"));
        assert_eq!(permission["bash"].as_str(), write.then_some("allow"));
        assert_eq!(config["share"], "disabled");
    }
}

fn output(stdout: &str, code: i32) -> process::CliOutput {
    process::CliOutput {
        stdout: stdout.into(),
        code: Some(code),
        stderr: "diagnostic".into(),
    }
}

#[test]
fn repeated_parts_are_replaced_and_distinct_parts_are_retained() {
    let stream = concat!(
        "{\"type\":\"text\",\"part\":{\"messageID\":\"old\",\"text\":\"narration\"}}\n",
        "{\"type\":\"text\",\"part\":{\"messageID\":\"answer\",\"id\":\"p1\",\"text\":\"{\\\"findings\\\":\"}}\n",
        "{\"type\":\"text\",\"part\":{\"messageID\":\"answer\",\"id\":\"p1\",\"text\":\"{\\\"findings\\\":\"}}\n",
        "{\"type\":\"text\",\"part\":{\"messageID\":\"answer\",\"id\":\"p2\",\"text\":\"[]}\"}}\n"
    );
    assert_eq!(
        events::answer(output(stream, 0)).unwrap(),
        r#"{"findings":[]}"#
    );
}

#[test]
fn error_after_text_never_becomes_a_successful_review() {
    let stream = "{\"type\":\"text\",\"text\":\"{\\\"findings\\\":[]}\"}\n{\"type\":\"error\",\"error\":{\"message\":\"model unavailable\"}}";
    assert!(
        events::answer(output(stream, 0))
            .unwrap_err()
            .to_string()
            .contains("model unavailable")
    );
    assert!(events::answer(output(r#"{"type":"text","text":"OK"}"#, 1)).is_err());
    assert!(events::answer(output(r#"{"type":"step_finish"}"#, 0)).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn real_subprocess_receives_workspace_stdin_and_local_model() {
    use std::os::unix::fs::PermissionsExt;
    let session = session::Session::new().await.unwrap();
    let script = session.dir.join("opencode-test");
    std::fs::write(
        &script,
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" > args
cat > brief
printf '%s' "$OPENCODE_CONFIG_CONTENT" > config
printf '%s\n' '{"type":"text","part":{"messageID":"m","id":"p","text":"{\"findings\":[]}"}}'
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = sweep(OpenCodeSweep {
        worktree: &session.dir,
        model: "ollama/qwen3:8b",
        effort: "",
        brief: "review this workspace",
        timeout: Duration::from_secs(5),
        binary: script.to_str(),
    })
    .await
    .unwrap();
    assert!(result.findings.is_empty());
    assert_eq!(
        std::fs::read_to_string(session.dir.join("brief")).unwrap(),
        "review this workspace"
    );
    let args = std::fs::read_to_string(session.dir.join("args")).unwrap();
    assert!(args.contains("ollama/qwen3:8b\n"));
    assert!(args.contains(session.dir.to_str().unwrap()));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(session.dir.join("config")).unwrap())
            .unwrap();
    let agent = config["agent"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(agent["permission"]["*"], "deny");
}

/// A local model's first call loads the weights, and the check must outlast
/// that. Measured cold on an 18 GB Ollama model: 95s to answer "OK", against
/// 12s warm — so the shared one-minute allowance killed the invocation that
/// was working, and the pre-check failed the whole run with it.
#[test]
fn the_signin_check_outlasts_a_cold_local_model_load() {
    let allowance = signin_timeout();
    assert!(
        allowance > signin::TIMEOUT,
        "OpenCode routes to local models, so it cannot share the hosted allowance"
    );
    assert!(
        allowance >= Duration::from_secs(190),
        "a measured 95s cold start needs headroom, not a coin flip: {allowance:?}"
    );
    // The reported number is the one actually waited, so the message cannot
    // claim a limit the check does not enforce.
    let timed_out = signin::classify(
        Err(ProviderError::Process(process::ProcessError::Timeout {
            what: "opencode CLI".into(),
            seconds: allowance.as_secs(),
            output: process::CliOutput {
                code: None,
                stdout: String::new(),
                stderr: String::new(),
            },
        })),
        signin_timeout().as_secs(),
    );
    assert_eq!(timed_out, signin::SignIn::TimedOut(allowance.as_secs()));
    assert!(
        timed_out
            .describe("opencode")
            .contains(&format!("{}s", allowance.as_secs()))
    );
}

/// Live acceptance for the cold start, against a real local model.
///
/// Ignored by default because it needs OpenCode, a configured local provider
/// and the weights on disk. Unload the model first (`ollama stop <model>`) or
/// it proves only the warm path:
///
/// ```text
/// BUGSLEUTH_LIVE_OPENCODE_MODEL=ollama/<model> \
///   cargo test -p bugsleuth-provider -- --ignored cold_local_model
/// ```
#[tokio::test]
#[ignore = "live: needs OpenCode and a real local model"]
async fn a_cold_local_model_reports_as_signed_in() {
    let model = std::env::var("BUGSLEUTH_LIVE_OPENCODE_MODEL")
        .expect("set BUGSLEUTH_LIVE_OPENCODE_MODEL to the local route to check");
    let started = std::time::Instant::now();
    let outcome = signin_for(&model, "", None).await;
    let waited = started.elapsed();
    assert_eq!(
        outcome,
        signin::SignIn::Working,
        "a cold local model must read as usable, not as a hang, after {waited:?}"
    );
    println!(
        "cold sign-in answered in {waited:?} (allowance {:?})",
        signin_timeout()
    );
}

/// The end-to-end shape of the bug, without needing a real model: a CLI that
/// says nothing for ninety seconds and then answers.
///
/// Under the shared one-minute allowance this was killed and reported as
/// "no answer within 60s, so a sweep would hang too", which failed the
/// pre-check and so the entire run. Ignored because it costs ninety seconds of
/// wall clock, which does not belong in the routine gate:
///
/// ```text
/// cargo test -p bugsleuth-provider -- --ignored a_slow_first_answer
/// ```
#[tokio::test]
#[ignore = "slow: waits out a 90-second first answer"]
async fn a_slow_first_answer_is_signed_in_not_timed_out() {
    const ANSWER: &str = r#"{"type":"text","part":{"messageID":"m","id":"p","text":"OK"}}"#;
    let session = session::Session::new().await.unwrap();
    #[cfg(windows)]
    let stub = {
        let path = session.dir.join("opencode.cmd");
        std::fs::write(
            &path,
            format!(
                "@echo off\r\nfindstr /R \".*\" > nul\r\npowershell -NoProfile -Command \"Start-Sleep -Seconds 90\" > nul\r\necho {ANSWER}\r\n"
            ),
        )
        .unwrap();
        path
    };
    #[cfg(not(windows))]
    let stub = {
        use std::os::unix::fs::PermissionsExt;
        let path = session.dir.join("opencode.sh");
        std::fs::write(
            &path,
            format!("#!/bin/sh\ncat >/dev/null\nsleep 90\nprintf '%s\n' '{ANSWER}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };

    let started = std::time::Instant::now();
    let outcome = signin_for("ollama/slow-to-load", "", stub.to_str()).await;
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(80),
        "the stub answered too early to be proving anything: {waited:?}"
    );
    assert_eq!(
        outcome,
        signin::SignIn::Working,
        "a first answer that arrives after the old one-minute allowance was killed instead of read"
    );
}
