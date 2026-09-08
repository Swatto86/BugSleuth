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
