//! Tests for the Claude adapter, in their own file only because the
//! adapter plus its tests crossed the hard line cap.

use super::super::claude::args::build_args;
use super::super::claude::*;
use bugsleuth_domain::{Lane, finding_schema};
use std::path::Path;
use std::time::Duration;

fn run<'a>(model: &'a str) -> Run<'a> {
    Run {
        effort: "",
        repo: Path::new("."),
        model,
        prompt: "",
        schema: finding_schema(),
        available: READ_ONLY_TOOLS,
        allowed: READ_ONLY_ALLOWED,
        denied: READ_ONLY_DENIED,
        max_turns: 12,
        timeout: Duration::from_secs(60),
        binary: None,
        api_key: None,
        session_id: Some(salvage::new_session_id()),
        resume: None,
    }
}

#[test]
fn read_only_sweeps_cannot_be_granted_write_tools() {
    let args = build_args(&run("sonnet"));
    let available = args
        .iter()
        .position(|a| a == "--tools")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    assert_eq!(available, Some(READ_ONLY_TOOLS));
    let index = args.iter().position(|a| a == "--disallowedTools");
    let denied = index.and_then(|i| args.get(i + 1)).map(String::as_str);
    assert_eq!(denied, Some(READ_ONLY_DENIED));
    assert!(!args.iter().any(|a| a.contains("dangerously-skip")));
}

#[test]
fn agent_enabled_sweeps_only_add_read_only_delegation_tools() {
    let mut spec = run("sonnet");
    spec.available = READ_ONLY_AGENT_TOOLS;
    spec.allowed = READ_ONLY_AGENT_ALLOWED;
    let args = build_args(&spec);
    let index = args.iter().position(|a| a == "--allowedTools");
    assert_eq!(
        index.and_then(|i| args.get(i + 1)).map(String::as_str),
        Some(READ_ONLY_AGENT_ALLOWED)
    );
    assert_eq!(
        args.iter()
            .position(|a| a == "--tools")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str),
        Some(READ_ONLY_AGENT_TOOLS)
    );
    assert_eq!(
        args.iter()
            .position(|a| a == "--disallowedTools")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str),
        Some(READ_ONLY_DENIED)
    );
}

#[test]
fn customizations_are_disabled_so_the_reviewed_repo_cannot_alter_the_review() {
    assert!(
        build_args(&run("sonnet"))
            .iter()
            .any(|a| a == "--safe-mode")
    );
}

#[test]
fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
    assert!(!build_args(&run("   ")).iter().any(|a| a == "--model"));
}

#[test]
fn the_schema_is_passed_as_one_argv_entry_not_shell_text() {
    let args = build_args(&run("sonnet"));
    let index = args.iter().position(|a| a == "--json-schema");
    let schema = index
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or("");
    let parsed: Value = serde_json::from_str(schema).unwrap_or(Value::Null);
    assert_eq!(parsed["type"], "object");
}

#[test]
fn an_empty_denylist_omits_the_flag_rather_than_passing_an_empty_value() {
    let mut spec = run("sonnet");
    spec.denied = "";
    assert!(!build_args(&spec).iter().any(|a| a == "--disallowedTools"));
}

#[test]
fn a_salvage_run_resumes_the_session_rather_than_starting_a_new_one() {
    // Without --resume the salvage would open a fresh conversation, which
    // knows nothing about the review it is meant to be transcribing - and
    // would answer confidently from nothing.
    let mut spec = run("sonnet");
    spec.resume = Some("session-abc");
    let args = build_args(&spec);
    let index = args.iter().position(|a| a == "--resume");
    assert_eq!(
        index.and_then(|i| args.get(i + 1)).map(String::as_str),
        Some("session-abc")
    );
    assert!(!args.iter().any(|a| a == "--session-id"));
    // And an ordinary sweep must never inherit a conversation.
    assert!(!build_args(&run("sonnet")).iter().any(|a| a == "--resume"));
}

#[test]
fn a_new_run_names_its_session_so_a_timeout_can_be_resumed() {
    let args = build_args(&run("sonnet"));
    let index = args.iter().position(|a| a == "--session-id");
    let session = index.and_then(|i| args.get(i + 1)).map(String::as_str);
    assert!(session.is_some_and(|id| id.len() == 36), "{args:?}");
}

#[cfg(windows)]
#[tokio::test]
async fn a_timed_out_run_resumes_the_same_session_once() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-claude-timeout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let stub = dir.join("claude.cmd");
    std::fs::write(
        &stub,
        "@echo off\r\n\
         findstr /R \".*\" > nul\r\n\
         echo %* | findstr /c:\"--resume\" > nul && goto resumed\r\n\
         echo %* > initial.txt\r\n\
         ping -n 10 127.0.0.1 > nul\r\n\
         exit /b 1\r\n\
         :resumed\r\n\
         echo %* > resumed.txt\r\n\
         echo %CLAUDE_CODE_RESUME_INTERRUPTED_TURN% > resumed-env.txt\r\n\
         echo {\"result\":\"recovered\",\"is_error\":false,\"session_id\":\"same-session\"}\r\n",
    )
    .expect("write CLI stub");
    let binary = stub.to_string_lossy().into_owned();

    let mut request = run("");
    request.repo = &dir;
    request.schema = serde_json::Value::Null;
    request.binary = Some(&binary);
    request.timeout = Duration::from_millis(500);
    let outcome = invoke::<Value>(request)
        .await
        .expect("resume should recover the answer");

    assert_eq!(outcome.result, Value::String("recovered".into()));
    assert!(outcome.salvaged);
    let value_after = |text: &str, flag: &str| {
        text.split_whitespace()
            .skip_while(|part| *part != flag)
            .nth(1)
            .map(str::to_string)
    };
    let initial = std::fs::read_to_string(dir.join("initial.txt")).expect("initial argv");
    let resumed = std::fs::read_to_string(dir.join("resumed.txt")).expect("resume argv");
    assert_eq!(
        value_after(&initial, "--session-id"),
        value_after(&resumed, "--resume")
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("resumed-env.txt"))
            .expect("resume environment")
            .trim(),
        "1"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[tokio::test]
async fn an_empty_successful_reply_resumes_the_same_session_once() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-claude-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let stub = dir.join("claude.cmd");
    std::fs::write(
        &stub,
        "@echo off\r\n\
         findstr /R \".*\" > nul\r\n\
         echo %* >> calls.txt\r\n\
         echo %* | findstr /c:\"--resume\" > nul && goto resumed\r\n\
         echo {\"result\":\"\",\"is_error\":false,\"session_id\":\"same-session\"}\r\n\
         exit /b 0\r\n\
         :resumed\r\n\
         echo {\"result\":\"\",\"structured_output\":{\"findings\":[]},\"is_error\":false,\"session_id\":\"same-session\"}\r\n",
    )
    .expect("write CLI stub");
    let binary = stub.to_string_lossy().into_owned();

    let mut request = run("");
    request.repo = &dir;
    request.binary = Some(&binary);
    let outcome = invoke::<RawFindings>(request)
        .await
        .expect("the empty answer should be recovered from its session");

    assert!(outcome.salvaged);
    assert_eq!(
        outcome.structured_result(),
        &serde_json::json!({"findings": []})
    );
    let calls = std::fs::read_to_string(dir.join("calls.txt")).expect("call log");
    assert_eq!(calls.lines().count(), 2, "recovery must run exactly once");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_wrong_shaped_successful_reply_resumes_the_same_session_once() {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-claude-wrong-shape-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    #[cfg(windows)]
    let (stub, script) = (
        dir.join("claude.cmd"),
        "@echo off\r\n\
         findstr /R \".*\" > nul\r\n\
         echo %* >> calls.txt\r\n\
         echo %* | findstr /c:\"--resume\" > nul && goto resumed\r\n\
         echo {\"result\":\"\",\"structured_output\":{},\"is_error\":false,\"session_id\":\"same-session\"}\r\n\
         exit /b 0\r\n\
         :resumed\r\n\
         echo {\"result\":\"\",\"structured_output\":{\"findings\":[{\"title\":\"known-finding\",\"severity\":\"medium\",\"file\":\"src/lib.rs\",\"line\":1,\"snippet\":\"known\",\"explanation\":\"known\",\"failure_scenario\":\"known\"}]},\"is_error\":false,\"session_id\":\"same-session\"}\r\n",
    );
    #[cfg(unix)]
    let (stub, script) = (
        dir.join("claude"),
        "#!/bin/sh\n\
         cat >/dev/null\n\
         printf '%s\\n' \"$*\" >> calls.txt\n\
         case \"$*\" in\n\
           *--resume*) printf '%s\\n' '{\"result\":\"\",\"structured_output\":{\"findings\":[{\"title\":\"known-finding\",\"severity\":\"medium\",\"file\":\"src/lib.rs\",\"line\":1,\"snippet\":\"known\",\"explanation\":\"known\",\"failure_scenario\":\"known\"}]},\"is_error\":false,\"session_id\":\"same-session\"}' ;;\n\
           *) printf '%s\\n' '{\"result\":\"\",\"structured_output\":{},\"is_error\":false,\"session_id\":\"same-session\"}' ;;\n\
         esac\n",
    );
    std::fs::write(&stub, script).expect("write CLI stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&stub)
            .expect("read stub permissions")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&stub, permissions).expect("make CLI stub executable");
    }
    let binary = stub.to_string_lossy().into_owned();

    let outcome = sweep(ClaudeSweep {
        repo: &dir,
        lane: Lane::Correctness,
        model: "",
        effort: "",
        use_agents: false,
        brief: "",
        timeout: Duration::from_secs(60),
        max_turns: 12,
        binary: Some(&binary),
        api_key: None,
    })
    .await
    .expect("the wrong-shaped answer should be recovered from its session");

    assert!(outcome.salvaged);
    assert_eq!(outcome.findings.findings.len(), 1);
    assert_eq!(outcome.findings.findings[0].title, "known-finding");
    let calls = std::fs::read_to_string(dir.join("calls.txt")).expect("call log");
    assert_eq!(calls.lines().count(), 2, "recovery must run exactly once");
    assert!(
        calls
            .lines()
            .nth(1)
            .is_some_and(|call| { call.contains("--resume") && call.contains("same-session") })
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_turn_budget_exhaustion_is_reported_as_its_own_failure_not_a_generic_one() {
    // It is the one failure worth resuming: the review may already be done
    // inside the conversation. Collapsing it into "exited non-zero" threw
    // the whole sweep away, which happened three times in one day.
    let exhausted = ProviderError::TurnsExhausted {
        vendor: VENDOR,
        session: Some("abc".into()),
    };
    assert!(exhausted.to_string().contains("turn budget"));
    // And it is not transient: retrying from scratch would hit the same
    // ceiling. Salvage is the answer, not another attempt.
    assert!(!exhausted.is_transient());
}

#[cfg(windows)]
#[tokio::test]
async fn a_salvage_is_never_itself_salvaged() {
    // The guard that stops a cycle: a resumed run that also runs out of
    // turns has said the conversation cannot answer, and a third attempt
    // would only spend more budget hearing it again. Exercised for real: a
    // stub CLI that always exhausts its turns, invoked with `resume` set,
    // must fail once and must not be resumed again.
    let dir = std::env::temp_dir().join(format!("bugsleuth-claude-salvage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let stub = dir.join("claude.cmd");
    std::fs::write(
        &stub,
        "@echo off\r\n\
         findstr /R \".*\" > nul\r\n\
         echo call >> calls.txt\r\n\
         echo {\"is_error\":true,\"subtype\":\"error_max_turns\",\"session_id\":\"same-session\"}\r\n\
         exit /b 1\r\n",
    )
    .expect("write CLI stub");
    let binary = stub.to_string_lossy().into_owned();

    let mut request = run("");
    request.repo = &dir;
    request.schema = serde_json::Value::Null;
    request.binary = Some(&binary);
    request.resume = Some("earlier-session");
    let outcome = invoke::<Value>(request).await;

    assert!(
        matches!(outcome, Err(ProviderError::TurnsExhausted { .. })),
        "an already-resuming run was salvaged again instead of failing plainly: {outcome:?}"
    );
    let calls = std::fs::read_to_string(dir.join("calls.txt")).expect("call log");
    assert_eq!(
        calls.lines().count(),
        1,
        "the CLI ran more than once, so the salvage was itself salvaged"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_invocation_with_no_schema_asks_for_none_rather_than_for_null() {
    // Applying fixes answers in prose. `--json-schema null` is not "no schema":
    // it constrains the reply to the JSON literal `null`, which would report an
    // empty account of work that really happened.
    let mut spec = run("sonnet");
    spec.schema = serde_json::Value::Null;
    let args = build_args(&spec);
    assert!(!args.iter().any(|a| a == "--json-schema"));
    assert!(!args.iter().any(|a| a == "null"));
    // The schema is the only thing dropped; everything else still applies.
    assert!(args.iter().any(|a| a == "--safe-mode"));
    assert!(args.iter().any(|a| a == "--allowedTools"));
}

#[test]
fn every_invocation_tells_the_cli_not_to_sign_the_commit() {
    // Without this the CLI adds `Co-Authored-By: Claude …` to every commit it
    // makes, and applying fixes asks it to commit per defect — so the tool
    // wrote attribution into a user's history as a side effect. Measured both
    // ways against a real repository before this was added.
    let args = build_args(&run("sonnet")).join(" ");
    assert!(args.contains("--settings"), "{args}");
    assert!(args.contains(r#"{"includeCoAuthoredBy":false}"#), "{args}");
}

/// The sign-in probe runs before any sweep, so it must use the same
/// customization boundary a real review does and must not run from the caller's
/// working directory. A stub CLI records the argv and cwd it was handed; the
/// probe is asserted to carry `--safe-mode` and to have run from somewhere
/// other than the process's own directory.
#[tokio::test]
async fn the_signin_probe_uses_safe_mode_and_a_private_directory() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-claude-signin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");

    #[cfg(windows)]
    let (stub, script) = (
        dir.join("claude.cmd"),
        format!(
            "@echo off\r\necho %* > \"{dir}\\args.txt\"\r\necho %CD% > \"{dir}\\cwd.txt\"\r\n",
            dir = dir.display()
        ),
    );
    #[cfg(unix)]
    let (stub, script) = (
        dir.join("claude"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"{dir}/args.txt\"\npwd > \"{dir}/cwd.txt\"\n",
            dir = dir.display()
        ),
    );
    std::fs::write(&stub, script).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).unwrap();
    }

    let _ = signin(Some(&stub.to_string_lossy())).await;

    let args = std::fs::read_to_string(dir.join("args.txt")).expect("argv captured");
    assert!(
        args.contains("--safe-mode"),
        "the sign-in probe did not disable customizations: {args}"
    );
    let probe_cwd = std::fs::read_to_string(dir.join("cwd.txt"))
        .expect("cwd captured")
        .trim()
        .to_string();
    let here = std::env::current_dir().expect("process cwd");
    let probe_canonical = std::fs::canonicalize(&probe_cwd).unwrap_or_default();
    let here_canonical = here.canonicalize().unwrap_or_default();
    assert_ne!(
        probe_canonical, here_canonical,
        "the sign-in probe ran from the caller's directory rather than a private one: {probe_cwd}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
