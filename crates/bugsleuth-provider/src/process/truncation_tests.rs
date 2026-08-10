//! Process-level proof that bounded output is never presented as complete.

use super::*;

fn shell() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        ("cmd.exe", vec!["/C".into()])
    } else {
        ("/bin/sh", vec!["-c".into()])
    }
}

fn emit(text: &str, wait: bool) -> String {
    if cfg!(windows) {
        format!(
            "<nul set /p ={text}{}",
            if wait {
                "&ping -n 30 127.0.0.1 >NUL"
            } else {
                "&exit /b 0"
            }
        )
    } else {
        format!("printf %s {text}{}", if wait { "; sleep 30" } else { "" })
    }
}

async fn run_at_cap(
    text: &str,
    cap: usize,
    timeout: Duration,
    wait: bool,
) -> Result<CliOutput, ProcessError> {
    let (binary, mut args) = shell();
    args.push(emit(text, wait));
    run_inner(
        Invocation {
            binary,
            args: &args,
            cwd: Path::new("."),
            stdin: None,
            env: &[],
            timeout,
            what: "capped test",
        },
        true,
        cap,
    )
    .await
}

#[tokio::test]
async fn output_over_the_cap_is_never_reported_as_complete() {
    let result = run_at_cap("earlyFINAL", 5, Duration::from_secs(30), false).await;
    let error = result.expect_err("overflow was reported as complete");
    assert_eq!(
        error.output().map(|output| output.stdout.as_str()),
        Some("early")
    );
    let diagnostic = error.to_string();
    let ProcessError::OutputTruncated {
        streams,
        limit,
        output,
        ..
    } = error
    else {
        panic!("overflow returned the wrong error: {error:?}");
    };
    assert_eq!(streams, "stdout");
    assert_eq!(limit, 5);
    assert_eq!(output.code, Some(0));
    assert_eq!(output.stdout, "early");
    assert!(!output.stdout.contains("FINAL"));
    assert!(diagnostic.contains("stdout"), "{diagnostic}");
    assert!(diagnostic.contains("5-byte"), "{diagnostic}");
    assert!(diagnostic.contains("only a prefix"), "{diagnostic}");
}

#[tokio::test]
async fn output_exactly_at_the_cap_is_complete() {
    let output = run_at_cap("12345678", 8, Duration::from_secs(30), false)
        .await
        .expect("exactly-at-cap output was called truncated");
    assert!(output.succeeded());
    assert_eq!(output.stdout, "12345678");
}

#[tokio::test]
async fn overflow_before_timeout_keeps_both_facts() {
    let result = run_at_cap("earlyFINAL", 5, Duration::from_secs(1), true).await;
    let error = result.expect_err("timeout overflow was reported as complete");
    let ProcessError::OutputTruncated {
        context, output, ..
    } = error
    else {
        panic!("overflow was hidden by a different error: {error:?}");
    };
    assert!(context.contains("timed out after 1s"), "{context}");
    assert_eq!(output.code, None);
    assert_eq!(output.stdout, "early");
}
