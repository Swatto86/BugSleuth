//! Codex's answer-file exception to incomplete event capture.

use super::*;

fn truncated(code: i32) -> Result<process::CliOutput, process::ProcessError> {
    Err(process::ProcessError::OutputTruncated {
        what: "codex CLI".into(),
        streams: "stdout",
        limit: 8,
        context: "when the process exited".into(),
        output: Box::new(process::CliOutput {
            code: Some(code),
            stdout: "event-prefix".into(),
            stderr: String::new(),
        }),
    })
}

#[test]
fn codex_only_accepts_truncation_when_the_answer_file_is_authoritative() {
    let path = std::env::temp_dir().join(format!(
        "bugsleuth-codex-truncated-answer-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, r#"{"findings":[]}"#).expect("write complete answer file");

    let answer = finish(truncated(0), &path).expect("successful answer file was rejected");
    assert_eq!(answer, r#"{"findings":[]}"#);
    assert!(
        matches!(
            finish(truncated(1), &path),
            Err(ProviderError::Process(
                process::ProcessError::OutputTruncated { .. }
            ))
        ),
        "a failed truncated event stream trusted the answer file"
    );

    let _ = std::fs::remove_file(path);
}
