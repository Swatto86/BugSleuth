//! Tests for the Codex review argv and recovery, in their own file because
//! the module plus its tests crossed the hard line cap.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::finding_schema;

use super::*;
use crate::codex::SHARED_FLAGS;
use crate::codex::scratch::Cleanup;

fn spec<'a>(model: &'a str) -> Invoke<'a> {
    Invoke {
        effort: "",
        dir: Path::new("."),
        model,
        brief: "",
        timeout: Duration::from_secs(60),
        binary: None,
        schema: finding_schema(),
    }
}

fn args_for(model: &str) -> Vec<String> {
    build_args(&spec(model), Path::new("s.json"), Path::new("a.json"))
}

#[test]
fn a_sweep_runs_read_only_and_ignores_the_reviewed_repos_own_config() {
    let joined = args_for("gpt-5.6-codex").join(" ");
    assert!(joined.contains("--sandbox read-only"));
    assert!(joined.contains("--ignore-user-config"));
    assert!(joined.contains("--ignore-rules"));
    assert!(!joined.contains("dangerously"));
}

#[test]
fn the_prompt_comes_from_stdin_so_a_long_brief_cannot_overflow_the_command_line() {
    assert_eq!(args_for("").last().map(String::as_str), Some("-"));
}

#[test]
fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
    assert!(!args_for("  ").iter().any(|a| a == "-m"));
}

#[test]
fn the_schema_and_answer_paths_are_passed_as_files_not_inline_json() {
    let args = args_for("");
    let after = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    assert_eq!(after("--output-schema"), Some("s.json"));
    assert_eq!(after("--output-last-message"), Some("a.json"));
}

#[test]
fn the_signin_probe_and_a_sweep_share_one_flag_list() {
    let parent = include_str!("../codex.rs");
    let sweep = include_str!("sweep.rs");
    assert!(
        parent.contains("SHARED_FLAGS"),
        "the sign-in probe must use SHARED_FLAGS"
    );
    assert!(
        sweep.contains("SHARED_FLAGS"),
        "build_args must use SHARED_FLAGS, or the check stops testing what a run does"
    );
}

#[test]
fn the_configuration_and_rules_of_the_host_and_the_repo_are_both_ignored() {
    assert!(SHARED_FLAGS.contains(&"--ignore-user-config"));
    assert!(SHARED_FLAGS.contains(&"--ignore-rules"));
    assert!(SHARED_FLAGS.contains(&"--skip-git-repo-check"));
}

#[test]
fn sweep_sessions_are_persisted_so_a_timeout_can_resume_them() {
    assert!(
        !args_for("").iter().any(|arg| arg == "--ephemeral"),
        "an ephemeral thread id cannot be resumed after the CLI is killed"
    );
}

#[test]
fn an_invocation_with_no_schema_names_no_schema_file() {
    let mut spec = spec("");
    spec.schema = serde_json::Value::Null;
    let args = build_args(&spec, Path::new("s.json"), Path::new("a.json")).join(" ");
    assert!(!args.contains("--output-schema"));
    assert!(args.contains("--output-last-message a.json"));
}

#[test]
fn a_named_model_and_effort_reach_the_cli() {
    let spec = Invoke {
        effort: "high",
        dir: Path::new("."),
        model: "gpt-5.6-codex",
        brief: "",
        timeout: Duration::from_secs(60),
        binary: None,
        schema: finding_schema(),
    };
    let args = build_args(&spec, Path::new("s.json"), Path::new("a.json"));
    assert!(args.windows(2).any(|w| w == ["-m", "gpt-5.6-codex"]));
    assert!(
        args.windows(2)
            .any(|w| w == ["-c", "model_reasoning_effort=\"high\""])
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

#[cfg(windows)]
#[tokio::test]
async fn cancelled_codex_invocation_removes_scratch() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-cancel-work-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the working directory");
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

    let created = {
        let call = invoke_text(Invoke {
            dir: &dir,
            model: "",
            effort: "",
            brief: "hello",
            timeout: Duration::from_secs(120),
            binary: Some(&stub),
            schema: serde_json::Value::Null,
        });
        tokio::pin!(call);
        let waited = tokio::time::timeout(Duration::from_secs(4), call.as_mut()).await;
        assert!(
            waited.is_err(),
            "the blocking stub should still have been running"
        );

        let created: Vec<_> = scratch_dirs()
            .into_iter()
            .filter(|d| !before.contains(d))
            .collect();
        assert!(!created.is_empty(), "no scratch directory was created");
        created
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
fn cleanup_guard_removes_the_directory_on_drop() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-cleanup-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the directory");
    std::fs::write(dir.join("schema.json"), "{}").expect("seed a file");
    {
        let _guard = Cleanup(dir.clone());
        assert!(dir.exists(), "precondition: the directory should exist");
    }
    assert!(
        !dir.exists(),
        "the guard did not remove the scratch directory when it dropped"
    );
}
