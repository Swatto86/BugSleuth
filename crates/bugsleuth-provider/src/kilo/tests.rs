//! Tests for the Kilo adapter, in their own file only because the module
//! plus its tests crossed the hard line cap.

use super::*;

fn spec<'a>(model: &'a str) -> KiloSweep<'a> {
    KiloSweep {
        effort: "",
        worktree: Path::new("/tmp/wt"),
        model,
        brief: "",
        timeout: Duration::from_secs(60),
        binary: None,
    }
}

/// Two Kilo processes must never overlap, whichever entry point started them.
///
/// Kilo processes share a mutable credential store; parallel same-provider
/// sweeps were removed for exactly this reason, but the diagnostic entry points
/// were left able to start a second `kilo run` beside a live one.
///
/// Two real sweeps against a stub that records when it starts and stops. Under
/// the guard the record reads start/end/start/end; without it, the two starts
/// come first.
#[tokio::test]
async fn kilo_operations_are_serialized() {
    let dir = scratch("serialized");
    let log = dir.join("order.txt");

    #[cfg(windows)]
    let stub = {
        let path = dir.join("kilo.cmd");
        std::fs::write(
            &path,
            format!(
                "@echo off\r\nfindstr /R \".*\" > nul\r\n>>\"{0}\" echo start\r\nping -n 2 127.0.0.1 > nul\r\n>>\"{0}\" echo end\r\nexit /b 1\r\n",
                log.display()
            ),
        )
        .expect("write stub");
        path
    };
    #[cfg(not(windows))]
    let stub = {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("kilo.sh");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ncat >/dev/null\necho start >> '{0}'\nsleep 1\necho end >> '{0}'\nexit 1\n",
                log.display()
            ),
        )
        .expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };
    let stub = stub.to_string_lossy().into_owned();

    // A real working directory: the sweep spawns the stub with `cwd` set to the
    // worktree, and a missing one fails before the process exists.
    let one = sweep(KiloSweep {
        binary: Some(&stub),
        worktree: &dir,
        ..spec("")
    });
    let two = sweep(KiloSweep {
        binary: Some(&stub),
        worktree: &dir,
        ..spec("")
    });
    let (first, second) = tokio::join!(one, two);
    assert!(first.is_err() && second.is_err(), "the stub always fails");

    let order: Vec<String> = std::fs::read_to_string(&log)
        .expect("the stub recorded nothing, so this test would prove nothing")
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        order,
        ["start", "end", "start", "end"],
        "two Kilo processes overlapped over one credential store"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The diagnostics refuse rather than queue behind an hour-long sweep.
///
/// Waiting would be safe but would read as a hung window: Check sign-in and the
/// model dropdown both go through Kilo, and a sweep holds the slot for tens of
/// minutes. Saying why is the honest answer.
#[tokio::test]
async fn kilo_diagnostics_refuse_while_an_operation_holds_the_slot() {
    // A discoverable binary, so this reaches the guard rather than stopping at
    // "the kilo CLI could not be found" — which is what every CI runner would
    // otherwise assert on, proving nothing about serialization. The path is
    // never executed: the guard refuses before anything is spawned.
    let dir = scratch("diagnostic-guard");
    let stub = dir.join(if cfg!(windows) { "kilo.cmd" } else { "kilo.sh" });
    std::fs::write(&stub, "").expect("write stub");

    let held = operation_guard().await;
    let checked = signin_for("", "", Some(&stub.to_string_lossy())).await;
    assert!(
        !checked.usable(),
        "the sign-in check ran a second Kilo process beside a live one"
    );
    assert!(
        format!("{checked:?}").contains("already running"),
        "the refusal does not say why: {checked:?}"
    );
    // Nothing is asserted about the slot being *free* afterwards: the lock is
    // process-wide, and the serialization test above holds it for its own two
    // sweeps, so any such assertion races with whichever test runs alongside.
    // Release is `MutexGuard`'s contract, not this test's subject.
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A scratch directory that is empty at the start of every run.
///
/// Cleared rather than merely created: a process id is reused eventually, and a
/// stale `kilo.jsonc` from an earlier run would be read as this run's fixture —
/// the test would then be asserting against a config it did not write.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-kilo-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}

#[test]
fn the_preflight_checks_the_agent_the_sweep_actually_runs_as() {
    // Not a comparison of two constants — since both sides read
    // `SWEEP_AGENT`, that would agree by construction and could never fail.
    // What must hold is that the preflight's *verdict* is about the agent
    // in the argv: a config hardening only that agent has to pass, and one
    // hardening only a different agent has to fail. Otherwise the check
    // could clear `ask` while the sweep ran as something unverified.
    let args = build_args(&spec("kilo/openai/gpt-5.6-sol"));
    let agent = args
        .iter()
        .position(|a| a == "--agent")
        .and_then(|i| args.get(i + 1))
        .expect("a sweep must name an agent")
        .clone();

    let denied = r#"{"*":"deny","read":"allow","glob":"allow","grep":"allow"}"#;
    let dir = scratch("agent-agreement");

    let matching = dir.join("kilo.jsonc");
    std::fs::write(
        &matching,
        format!(r#"{{"agent":{{"{agent}":{{"permission":{denied}}}}}}}"#),
    )
    .expect("write config");
    assert_eq!(
        preflight::gap_in(&[matching]),
        None,
        "hardening the agent the sweep runs as should satisfy the preflight"
    );

    // The same config with only the agent name changed. `is_some()` alone would
    // also pass for an unwritten, unreadable, or unparseable file — every one of
    // which is the wrong reason — so the gap has to name the agent it looked for
    // and say it was the permission block that was missing.
    let other = dir.join("other.jsonc");
    std::fs::write(
        &other,
        format!(r#"{{"agent":{{"not-{agent}":{{"permission":{denied}}}}}}}"#),
    )
    .expect("write config");
    let gap = preflight::gap_in(&[other])
        .expect("hardening a different agent must not satisfy the preflight");
    assert!(
        gap.contains(&agent) && gap.contains("permission"),
        "the refusal must be about the sweep's own agent, not an unreadable file: {gap}"
    );
}

#[test]
fn the_model_id_says_which_account_a_sweep_will_spend_from() {
    // The same model reached three ways bills to three different places.
    assert_eq!(route_of("kilo/z-ai/glm-5"), Route::Gateway);
    assert_eq!(route_of("openrouter/z-ai/glm-5"), Route::OpenRouter);
    assert_eq!(route_of("openai/gpt-5"), Route::OpenAi);
    assert_eq!(route_of("ollama/llama3"), Route::Ollama);
}

#[test]
fn an_id_with_no_prefix_leaves_the_choice_to_kilo() {
    assert_eq!(route_of(""), Route::Configured);
    assert_eq!(route_of("  "), Route::Configured);
    assert_eq!(route_of("some-model"), Route::Configured);
}

#[test]
fn every_route_can_be_described_to_a_person() {
    for route in [
        Route::Gateway,
        Route::OpenRouter,
        Route::OpenAi,
        Route::Ollama,
        Route::Configured,
    ] {
        assert!(!route.describe().is_empty());
    }
}

#[test]
fn external_plugins_are_skipped_so_the_machine_cannot_change_the_review() {
    assert!(build_args(&spec("")).iter().any(|a| a == "--pure"));
}

#[test]
fn external_project_skills_are_disabled() {
    let env = sweep_environment();
    assert_eq!(
        env.as_slice(),
        &[(
            "KILO_DISABLE_EXTERNAL_SKILLS".to_string(),
            "true".to_string()
        )]
    );
}

#[test]
fn the_working_directory_is_pinned_explicitly() {
    let args = build_args(&spec(""));
    let dir = args
        .iter()
        .position(|a| a == "--dir")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    assert_eq!(dir, Some("/tmp/wt"));
}

#[test]
fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
    assert!(!build_args(&spec("   ")).iter().any(|a| a == "-m"));
}

#[cfg(windows)]
#[tokio::test]
async fn a_timed_out_sweep_resumes_the_session_reported_by_kilo() {
    let dir = scratch("timeout-recovery");
    let stub = dir.join("kilo.cmd");
    std::fs::write(
        &stub,
        "@echo off\r\n\
         echo %* | findstr /c:\"--session kilo-session-123\" > nul && goto resumed\r\n\
         echo {\"type\":\"step_start\",\"sessionID\":\"kilo-session-123\"}\r\n\
         ping -n 10 127.0.0.1 > nul\r\n\
         exit /b 1\r\n\
         :resumed\r\n\
         echo {\"type\":\"text\",\"sessionID\":\"kilo-session-123\",\"part\":{\"messageID\":\"m1\",\"text\":\"{\\\"findings\\\":[]}\"}}\r\n",
    )
    .expect("write CLI stub");
    let binary = stub.to_string_lossy().into_owned();

    let result = sweep(KiloSweep {
        worktree: &dir,
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
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[tokio::test]
async fn kilos_native_timeout_exit_resumes_the_session_it_reported() {
    let dir = scratch("native-timeout-recovery");
    let stub = dir.join("kilo.cmd");
    std::fs::write(
        &stub,
        "@echo off\r\n\
         echo %* | findstr /c:\"--session kilo-session-native\" > nul && goto resumed\r\n\
         echo {\"type\":\"step_start\",\"sessionID\":\"kilo-session-native\"}\r\n\
         exit /b 124\r\n\
         :resumed\r\n\
         echo {\"type\":\"text\",\"sessionID\":\"kilo-session-native\",\"part\":{\"messageID\":\"m1\",\"text\":\"{\\\"findings\\\":[]}\"}}\r\n",
    )
    .expect("write CLI stub");
    let binary = stub.to_string_lossy().into_owned();

    let result = sweep(KiloSweep {
        worktree: &dir,
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
    let _ = std::fs::remove_dir_all(&dir);
}
