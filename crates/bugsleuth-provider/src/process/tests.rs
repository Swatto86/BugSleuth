//! Tests for the shared spawner, in their own file only because the module
//! plus its tests crossed the hard line cap.

use super::*;

fn shell() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        ("cmd.exe", vec!["/C".into()])
    } else {
        ("/bin/sh", vec!["-c".into()])
    }
}

#[tokio::test]
async fn captures_stdout_and_a_zero_exit_code() {
    let (binary, mut args) = shell();
    args.push("echo hello".into());
    let output = run(Invocation {
        binary,
        args: &args,
        cwd: Path::new("."),
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(30),
        what: "test",
    })
    .await;
    let output = output.unwrap_or_else(|e| panic!("spawn failed: {e}"));
    assert!(output.succeeded(), "expected success, got {output:?}");
    assert!(
        output.stdout.contains("hello"),
        "stdout was {:?}",
        output.stdout
    );
}

#[tokio::test]
async fn reports_a_nonzero_exit_code_rather_than_erroring() {
    let (binary, mut args) = shell();
    args.push("exit 3".into());
    let output = run(Invocation {
        binary,
        args: &args,
        cwd: Path::new("."),
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(30),
        what: "test",
    })
    .await;
    let output = output.unwrap_or_else(|e| panic!("spawn failed: {e}"));
    assert_eq!(output.code, Some(3));
    assert!(!output.succeeded());
}

#[tokio::test]
async fn kills_a_child_that_outlives_its_timeout() {
    let (binary, mut args) = shell();
    // Both shells understand a long-running ping loopback wait on Windows;
    // sleep elsewhere.
    args.push(if cfg!(windows) {
        "ping -n 30 127.0.0.1 >NUL".into()
    } else {
        "sleep 30".to_string()
    });
    let result = run(Invocation {
        binary,
        args: &args,
        cwd: Path::new("."),
        stdin: None,
        env: &[],
        timeout: Duration::from_millis(400),
        what: "slow test",
    })
    .await;
    assert!(
        matches!(result, Err(ProcessError::Timeout { .. })),
        "expected a timeout, got {result:?}"
    );
}

/// A scratch directory that is empty at the start of every run, so a marker
/// left by an earlier run cannot be read as this one's evidence.
#[cfg(windows)]
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-process-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}

/// A shim that starts something which outlives it, exactly as an npm `.cmd`
/// starts the vendor CLI.
///
/// The grandchild announces itself, waits, then records that it was still
/// alive. `ping` is the batch sleep — `timeout` needs a console this has not
/// got — and `start /b` detaches the grandchild so killing the shim alone
/// leaves it running.
///
/// Everything is named relative to the working directory the invocation pins,
/// because `start` reads a quoted path as a window title and there is no
/// arrangement of quotes that survives both it and the shim.
#[cfg(windows)]
fn shim_that_starts_a_survivor(dir: &std::path::Path) -> String {
    std::fs::write(
        dir.join("grandchild.cmd"),
        "@echo off\r\necho started > started.txt\r\nping -n 6 127.0.0.1 > nul\r\necho survived > \
         survived.txt\r\n",
    )
    .expect("write grandchild");
    std::fs::write(
        dir.join("stub.cmd"),
        "@echo off\r\nstart /b grandchild.cmd\r\nping -n 30 127.0.0.1 > nul\r\n",
    )
    .expect("write stub");
    dir.join("stub.cmd").to_string_lossy().into_owned()
}

/// Wait past the point where the grandchild would have finished, then say
/// whether it was killed.
///
/// The `started` half is the known-present guard: without it, a stub that never
/// managed to start anything would pass by having nothing left to kill.
#[cfg(windows)]
async fn grandchild_was_killed(dir: &std::path::Path) -> bool {
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert!(
        dir.join("started.txt").exists(),
        "the grandchild never ran, so this proves nothing about killing it"
    );
    !dir.join("survived.txt").exists()
}

/// Windows only, because it is the only platform where a shim stands between us
/// and the CLI — and the only one [`kill_tree`] does anything on.
#[cfg(windows)]
#[tokio::test]
async fn a_timeout_kills_what_the_child_started_and_not_only_the_child() {
    // The defect this exists for: `start_kill` reached the `cmd.exe` shim and
    // nothing below it, so a sweep killed at its timeout left the vendor CLI
    // running against a worktree about to be deleted.
    let dir = scratch("tree-kill");
    let stub = shim_that_starts_a_survivor(&dir);

    let result = run(Invocation {
        binary: &stub,
        args: &[],
        cwd: &dir,
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(2),
        what: "tree test",
    })
    .await;
    assert!(
        matches!(result, Err(ProcessError::Timeout { .. })),
        "expected a timeout, got {result:?}"
    );

    assert!(
        grandchild_was_killed(&dir).await,
        "the grandchild outlived the process we killed"
    );
}

/// The other way a sweep ends: nobody waits for it any more.
#[cfg(windows)]
#[tokio::test]
async fn abandoning_the_call_kills_the_tree_rather_than_just_the_child() {
    // A cancelled run drops this future. `kill_on_drop` then kills the shim and
    // nothing under it, which is how a killed run left a CLI holding the
    // worktree it was reviewing. The guard has to run *before* the child it
    // guards, so moving its `let` above the child's would break this.
    let dir = scratch("tree-kill-cancel");
    let stub = shim_that_starts_a_survivor(&dir);

    {
        let call = run(Invocation {
            binary: &stub,
            args: &[],
            cwd: &dir,
            stdin: None,
            env: &[],
            // Long enough that the timeout is never what ends this.
            timeout: Duration::from_secs(120),
            what: "cancelled test",
        });
        tokio::pin!(call);
        let waited = tokio::time::timeout(Duration::from_secs(2), call.as_mut()).await;
        assert!(waited.is_err(), "the call should still have been running");
        // Dropped here, with the child alive — the cancellation path.
    }

    assert!(
        grandchild_was_killed(&dir).await,
        "the grandchild outlived the abandoned call"
    );
}

#[test]
fn a_vendors_own_credential_dump_is_redacted_from_previewed_output() {
    // The defect a user hit: Kilo's CLI printed its OAuth refresh and access
    // tokens into stderr on a credential-store error, and that stderr becomes a
    // report's "NOT SWEPT" reason — shown in the window and written to disk under
    // runs/. A JWT-shaped token must never survive into any previewed output.
    let leak = "Error: Unexpected error\n\
         \"refresh\":\"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIn0.s3cr3t_sig-Value\",\
         \"access\":\"eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJpZCI6NDJ9.another-Sig_here\"";
    let safe = preview(leak, 2000);
    assert!(!safe.contains("eyJhbGci"), "a JWT header survived: {safe}");
    assert!(!safe.contains("eyJ0eXAi"), "a JWT header survived: {safe}");
    assert!(
        !safe.contains("s3cr3t_sig-Value") && !safe.contains("another-Sig_here"),
        "a token signature survived: {safe}"
    );
    // The non-secret context is kept, so the failure is still diagnosable, and
    // the redaction is visible rather than a silent gap.
    assert!(
        safe.contains("Unexpected error"),
        "context was lost: {safe}"
    );
    assert!(
        safe.contains("redacted"),
        "nothing marks the redaction: {safe}"
    );
}

/// A prompt that never reached the child must not be reported as a sweep.
///
/// The defect: `let _ = stdin.write_all(&bytes).await`. A failed write was
/// discarded, so a truncated or empty prompt looked exactly like a normal
/// run — the model answered whatever it had received, possibly nothing, and
/// that was presented as a review of the code.
///
/// **Structural, and honest about it.** The behavioural version needs a
/// child that closes stdin mid-write, which is a race rather than a test.
/// What can be checked deterministically is that the result is still
/// consumed: the fix was shipped claiming a test it did not have, which an
/// independent audit found by reverting it and watching every existing test
/// pass. This is the assertion that would have failed.
#[test]
fn the_result_of_writing_the_prompt_is_still_consumed() {
    let source = include_str!("../process.rs");
    let code = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(before, _)| before);

    assert!(
        !code.contains("let _ = stdin.write_all"),
        "the prompt write is discarded again, so a prompt that never arrived \
         would be reported as a completed sweep"
    );
    assert!(
        code.contains("fed.map_err"),
        "nothing consumes the outcome of writing the prompt, so a failed \
         write cannot become a failed invocation"
    );
}
