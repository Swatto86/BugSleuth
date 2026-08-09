//! Tests for the Codex adapter, in their own file only because the module
//! plus its tests crossed the hard line cap.

use super::*;

fn spec<'a>(model: &'a str, sandbox: Sandbox) -> Invoke<'a> {
    Invoke {
        effort: "",
        dir: Path::new("."),
        model,
        brief: "",
        timeout: Duration::from_secs(60),
        binary: None,
        schema: finding_schema(),
        sandbox,
    }
}

fn args_for(model: &str, sandbox: Sandbox) -> Vec<String> {
    build_args(
        &spec(model, sandbox),
        Path::new("s.json"),
        Path::new("a.json"),
    )
}

#[test]
fn a_sweep_runs_read_only_and_ignores_the_reviewed_repos_own_config() {
    let joined = args_for("gpt-5.6-codex", Sandbox::ReadOnly).join(" ");
    assert!(joined.contains("--sandbox read-only"));
    assert!(joined.contains("--ignore-user-config"));
    assert!(joined.contains("--ignore-rules"));
    assert!(!joined.contains("dangerously"));
}

#[test]
fn an_apply_may_write_because_it_has_to_edit_files() {
    let joined = args_for("", Sandbox::WorkspaceWrite).join(" ");
    assert!(joined.contains("--sandbox workspace-write"));
    // Still never the escape hatch, even when writing is allowed.
    assert!(!joined.contains("dangerously"));
}

#[test]
fn the_prompt_comes_from_stdin_so_a_long_brief_cannot_overflow_the_command_line() {
    let args = args_for("", Sandbox::ReadOnly);
    assert_eq!(args.last().map(String::as_str), Some("-"));
}

#[test]
fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
    assert!(!args_for("  ", Sandbox::ReadOnly).iter().any(|a| a == "-m"));
}

#[test]
fn the_schema_and_answer_paths_are_passed_as_files_not_inline_json() {
    let args = args_for("", Sandbox::ReadOnly);
    let after = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    assert_eq!(after("--output-schema"), Some("s.json"));
    assert_eq!(after("--output-last-message"), Some("a.json"));
}

/// A sign-in check must invoke the CLI the way a sweep does.
///
/// Otherwise it is not checking the sweep: it can pass while every real run
/// fails, or — what actually happened — fail while runs are fine. The first
/// version dropped `--skip-git-repo-check`, so Codex refused with "Not
/// inside a trusted directory" and a working session was reported as
/// unusable, pointing the reader at the wrong problem entirely.
#[test]
fn the_signin_probe_and_a_sweep_share_one_flag_list() {
    let source = include_str!("../codex.rs");
    let code = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(before, _)| before);
    assert_eq!(
        code.matches("SHARED_FLAGS").count(),
        3,
        "the sign-in probe and build_args must both use SHARED_FLAGS, or they \
             can drift and the check stops testing what a run does"
    );
}

/// The two flags that keep a review reproducible and unsteerable.
///
/// Without them the invocation picks up the machine's own Codex
/// configuration and the reviewed repository's rules — and the reviewed
/// repository is untrusted input whose text can address the model directly.
#[test]
fn the_configuration_and_rules_of_the_host_and_the_repo_are_both_ignored() {
    assert!(SHARED_FLAGS.contains(&"--ignore-user-config"));
    assert!(SHARED_FLAGS.contains(&"--ignore-rules"));
    assert!(SHARED_FLAGS.contains(&"--skip-git-repo-check"));
}

#[test]
fn sweep_sessions_are_persisted_so_a_timeout_can_resume_them() {
    let sweep = args_for("", Sandbox::ReadOnly);
    assert!(
        !sweep.iter().any(|arg| arg == "--ephemeral"),
        "an ephemeral thread id cannot be resumed after the CLI is killed"
    );
    assert!(
        args_for("", Sandbox::WorkspaceWrite)
            .iter()
            .any(|arg| arg == "--ephemeral"),
        "write-capable apply sessions are not recoverable and need not persist"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn a_timed_out_sweep_resumes_the_thread_reported_by_the_cli() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-codex-timeout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let stub = dir.join("codex.cmd");
    std::fs::write(
        &stub,
        "@echo off\r\n\
         echo %* | findstr /c:\"resume codex-thread-123\" > nul && goto resumed\r\n\
         echo {\"type\":\"thread.started\",\"thread_id\":\"codex-thread-123\"}\r\n\
         ping -n 10 127.0.0.1 > nul\r\n\
         exit /b 1\r\n\
         :resumed\r\n\
         echo %* > resumed.txt\r\n\
         :args\r\n\
         if \"%~1\"==\"\" exit /b 2\r\n\
         if \"%~1\"==\"--output-last-message\" goto answer\r\n\
         shift\r\n\
         goto args\r\n\
         :answer\r\n\
         >\"%~2\" echo {\"findings\":[]}\r\n\
         echo {\"type\":\"turn.completed\"}\r\n",
    )
    .expect("write CLI stub");
    let binary = stub.to_string_lossy().into_owned();

    let result = sweep(CodexSweep {
        repo: &dir,
        model: "",
        effort: "",
        brief: "review",
        timeout: Duration::from_millis(500),
        binary: Some(&binary),
    })
    .await
    .expect("resume should recover the answer");

    assert!(result.findings.findings.is_empty());
    assert!(result.salvaged);
    let resumed = std::fs::read_to_string(dir.join("resumed.txt")).expect("resume argv");
    assert!(resumed.contains("resume codex-thread-123"), "{resumed}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[tokio::test]
async fn a_transient_failed_turn_resumes_the_thread_reported_by_the_cli() {
    let dir =
        std::env::temp_dir().join(format!("bugsleuth-codex-transient-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let stub = dir.join("codex.cmd");
    std::fs::write(
        &stub,
        "@echo off\r\n\
         echo %* | findstr /c:\"resume codex-thread-transient\" > nul && goto resumed\r\n\
         echo {\"type\":\"thread.started\",\"thread_id\":\"codex-thread-transient\"}\r\n\
         echo {\"type\":\"turn.failed\",\"error\":{\"message\":\"server overloaded\"}}\r\n\
         exit /b 1\r\n\
         :resumed\r\n\
         echo %* > resumed.txt\r\n\
         :args\r\n\
         if \"%~1\"==\"\" exit /b 2\r\n\
         if \"%~1\"==\"--output-last-message\" goto answer\r\n\
         shift\r\n\
         goto args\r\n\
         :answer\r\n\
         >\"%~2\" echo {\"findings\":[]}\r\n\
         echo {\"type\":\"turn.completed\"}\r\n",
    )
    .expect("write CLI stub");
    let binary = stub.to_string_lossy().into_owned();

    let result = sweep(CodexSweep {
        repo: &dir,
        model: "",
        effort: "",
        brief: "review",
        timeout: Duration::from_secs(5),
        binary: Some(&binary),
    })
    .await
    .expect("resume should recover the answer");

    assert!(result.findings.findings.is_empty());
    assert!(result.salvaged);
    let resumed = std::fs::read_to_string(dir.join("resumed.txt")).expect("resume argv");
    assert!(
        resumed.contains("resume codex-thread-transient"),
        "{resumed}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cleanup_guard_removes_the_directory_on_drop() {
    // The scratch guard's whole job: whatever exit path invoke_text takes — a
    // normal return, an early `?`, or a dropped (cancelled) future — the scratch
    // directory goes with it. A plain drop is every one of those.
    let dir = std::env::temp_dir().join(format!("bugsleuth-cleanup-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the directory");
    std::fs::write(dir.join("schema.json"), "{}").expect("seed a file");
    {
        let _guard = Cleanup(dir.clone());
        assert!(dir.exists(), "precondition: the directory should exist");
    } // the guard drops here
    assert!(
        !dir.exists(),
        "the guard did not remove the scratch directory when it dropped"
    );
}

/// Cancelling a sweep drops invoke_text's future while it awaits the CLI; the
/// scratch directory it created must not be left behind. Windows only, because
/// it reuses the same blocking `.cmd` shim the process tests do.
#[cfg(windows)]
#[tokio::test]
async fn cancelled_codex_invocation_removes_scratch() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-cancel-work-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the working directory");
    // A stub that blocks, so the future is still awaiting when it is dropped.
    let stub = dir.join("stub.cmd");
    std::fs::write(&stub, "@echo off\r\nping -n 30 127.0.0.1 > nul\r\n").expect("write the stub");
    let stub = stub.to_string_lossy().into_owned();

    let scratch_dirs = || -> Vec<PathBuf> {
        std::fs::read_dir(std::env::temp_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("bugsleuth-codex-"))
            })
            .collect()
    };
    let before = scratch_dirs();

    // The future is owned inside this block and dropped at its end — the
    // cancellation. `tokio::pin!` rebinds the name to a `Pin<&mut _>`, so an
    // explicit `drop` of that would free only the reference, not the future.
    let created = {
        let call = invoke_text(Invoke {
            dir: &dir,
            model: "",
            effort: "",
            brief: "hello",
            timeout: Duration::from_secs(120),
            binary: Some(&stub),
            schema: serde_json::Value::Null,
            sandbox: Sandbox::ReadOnly,
        });
        tokio::pin!(call);
        let waited = tokio::time::timeout(Duration::from_secs(4), call.as_mut()).await;
        assert!(
            waited.is_err(),
            "the blocking stub should still have been running"
        );

        // A scratch directory must actually have been created, or the test
        // proves nothing by cleaning up nothing.
        let created: Vec<_> = scratch_dirs()
            .into_iter()
            .filter(|d| !before.contains(d))
            .collect();
        assert!(!created.is_empty(), "no scratch directory was created");
        created
        // the future drops here, with the child still alive: the cancel path.
    };
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        created.iter().all(|d| !d.exists()),
        "a scratch directory survived cancellation: {:?}",
        created.iter().filter(|d| d.exists()).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_invocation_with_no_schema_names_no_schema_file() {
    // No schema file is written when none was asked for, so naming one would
    // point the CLI at a path that does not exist and fail the whole run.
    let mut spec = spec("", Sandbox::WorkspaceWrite);
    spec.schema = serde_json::Value::Null;
    let args = build_args(&spec, Path::new("s.json"), Path::new("a.json")).join(" ");
    assert!(!args.contains("--output-schema"));
    // The final message still comes back through a file rather than the events.
    assert!(args.contains("--output-last-message a.json"));
}

#[test]
fn an_invocation_that_must_write_does_not_also_ignore_the_machines_config() {
    // Measured against the real CLI: `--ignore-user-config` makes it reject
    // every patch as "writing is blocked by read-only sandbox" no matter what
    // `--sandbox` says. Keeping both flags produced an invocation that ran, cost
    // quota, described the change it wanted to make — and changed nothing.
    let writing = args_for("", Sandbox::WorkspaceWrite).join(" ");
    assert!(!writing.contains("--ignore-user-config"));
    // The repository under review still cannot supply its own execution policy.
    assert!(writing.contains("--ignore-rules"));
    assert!(writing.contains("--sandbox workspace-write"));

    // A read-only sweep is unaffected and keeps the isolation.
    assert!(
        args_for("", Sandbox::ReadOnly)
            .join(" ")
            .contains("--ignore-user-config")
    );
}
