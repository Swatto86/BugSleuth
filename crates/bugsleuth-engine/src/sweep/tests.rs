use super::*;

#[test]
fn a_bare_model_name_means_claude_so_the_common_case_stays_short() {
    assert_eq!(Vendor::parse("sonnet"), (Vendor::Claude, "sonnet"));
}

#[test]
fn a_failed_sweeps_reason_is_scrubbed_of_credentials_before_it_is_stored() {
    // A vendor CLI can print its own OAuth tokens in an error — Kilo dumped a
    // refresh and access token on a credential-store failure — and that error
    // becomes the report's stored, displayed "NOT SWEPT" reason. The provider's
    // `redact_secrets` is unit-tested; this is the wiring test that the error
    // arm actually applies it, so a token cannot reach a report whichever stream
    // the CLI put it on. A source scan because the alternative is a real failing
    // CLI, and the anchor below is known-present so the scan cannot go vacuous.
    let source = include_str!("../sweep.rs");
    // The known-present anchor keeps the scan honest: if `not_swept` is gone the
    // check fails loudly rather than passing on nothing.
    assert!(
        source.contains("not_swept("),
        "the sweep no longer builds a NOT SWEPT reason; this check needs rewriting"
    );
    assert!(
        source.contains("not_swept(redact_secrets("),
        "a failed sweep's reason is stored without redacting credentials from it"
    );
}

#[test]
fn a_vendor_prefix_selects_that_vendor() {
    assert_eq!(
        Vendor::parse("codex:gpt-5.6-codex"),
        (Vendor::Codex, "gpt-5.6-codex")
    );
    assert_eq!(Vendor::parse("claude:opus"), (Vendor::Claude, "opus"));
}

#[test]
fn selected_providers_are_checked_once_before_lane_work() {
    let models = vec![
        "sonnet".to_string(),
        "claude:opus".to_string(),
        "kilo:kilo/zai-coding/glm-5.2".to_string(),
        "kilo:openrouter/another".to_string(),
    ];
    assert_eq!(
        precheck::vendors_for(&models),
        vec![Vendor::Claude, Vendor::Kilo]
    );
}

#[test]
fn provider_precheck_failures_are_reported_together() {
    use bugsleuth_provider::signin::SignIn;

    let credential = "eyJsecret.payload.signature";
    let error = precheck::summarize(vec![
        (
            Vendor::Claude,
            SignIn::Failed(format!("run `claude login`; token={credential}")),
        ),
        (Vendor::Kilo, SignIn::TimedOut(60)),
    ])
    .expect_err("two unusable providers passed");
    assert!(error.contains("before any lane started"), "{error}");
    assert!(error.contains("claude login"), "{error}");
    assert!(error.contains("kilo"), "{error}");
    assert!(error.contains("60s"), "{error}");
    assert!(!error.contains(credential), "{error}");
    assert!(error.contains("<redacted-credential>"), "{error}");
}

#[test]
fn kilo_is_selected_by_prefix_and_needs_isolation_because_it_has_no_read_only_mode() {
    assert_eq!(
        Vendor::parse("kilo:anthropic/claude-sonnet-4-5"),
        (Vendor::Kilo, "anthropic/claude-sonnet-4-5")
    );
    assert!(Vendor::Kilo.needs_isolation());
    assert!(!Vendor::Claude.needs_isolation());
    assert!(!Vendor::Codex.needs_isolation());
}

#[test]
fn only_kilo_needs_the_schema_spelled_out_in_its_prompt() {
    assert!(!Vendor::Kilo.enforces_schema());
    assert!(Vendor::Claude.enforces_schema());
    assert!(Vendor::Codex.enforces_schema());
}

#[test]
fn supported_vendors_get_their_own_agent_wording_and_kilo_gets_none() {
    let claude = super::agents::instruction(Vendor::Claude, "sonnet").unwrap_or_default();
    let codex = super::agents::instruction(Vendor::Codex, "").unwrap_or_default();
    assert!(claude.contains("foreground Explore subagents"), "{claude}");
    assert!(
        claude.contains("Do not use Workflow, background agents, or delayed wakeups"),
        "{claude}"
    );
    assert!(codex.contains("Codex subagents"), "{codex}");
    assert_eq!(super::agents::instruction(Vendor::Kilo, ""), None);
}

#[tokio::test]
async fn a_kilo_sweep_stops_at_the_preflight_before_doing_any_work() {
    // Whichever way this machine is configured, a Kilo sweep must consult
    // the preflight before discovering a provider or building a worktree.
    // The two outcomes are asserted against each other rather than against
    // a fixed expectation, because the honest answer depends on the config:
    // a refusal must name the open tool, and a pass must mean the config
    // really denies all host capabilities. Both halves of `permission_gap`
    // are tested directly, against written configs, in the provider crate.
    let report = run(Request {
        repo: Path::new("."),
        lane: Lane::Security,
        model: "kilo:some/model",
        scope: None,
        effort: "",
        max_turns: 1,
        timeout: Duration::from_secs(1),
        api_key: None,
        binary: None,
    })
    .await;

    match (kilo::preflight::permission_gap(), report.status) {
        (Some(gap), Status::NotSwept { reason }) => assert!(
            reason.contains(&gap),
            "the refusal did not carry the permission gap: {reason}"
        ),
        (Some(gap), other) => {
            panic!("permissions are open ({gap}) but the sweep was not refused: {other:?}")
        }
        // Permissions safe: the preflight is satisfied and the sweep proceeds
        // to the next failure, which without a Kilo binary is a real one.
        (None, _) => {}
    }
}

/// A stand-in CLI that fails, printing its agent instruction and argv.
///
/// Both are read from the real child process boundary rather than rebuilt in a
/// test. Stdin is consumed before exit, or the prompt may be undelivered.
fn echoing_stub(name: &str) -> String {
    let dir = std::env::temp_dir()
        .join("bugsleuth-sweep-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");

    #[cfg(windows)]
    let path = {
        let path = dir.join("stub.cmd");
        std::fs::write(
            &path,
            "@echo off\r\necho %* | findstr /C:\"--model sonnet\" > nul && echo model=sonnet 1>&2\r\nfindstr /C:\"subagent\" 1>&2\r\necho %* | findstr /C:\"--effort ultracode\" > nul && echo --effort ultracode 1>&2\r\necho %* 1>&2\r\nexit /b 1\r\n",
        )
        .expect("write stub");
        path
    };
    #[cfg(not(windows))]
    let path = {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("stub.sh");
        std::fs::write(
            &path,
            "#!/bin/sh\ncase \" $* \" in *\" --model sonnet \"*) echo model=sonnet >&2;; esac\ngrep subagent >&2\ncase \" $* \" in *\"--effort ultracode\"*) echo --effort ultracode >&2;; esac\necho \"$@\" >&2\nexit 1\n",
        )
        .expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };
    path.to_string_lossy().into_owned()
}

/// A stub that answers like a Claude sweep that found nothing.
///
/// The failure path alone cannot prove the successful `LaneReport` initializer
/// uses the resolved label; there are two of them, and both had to be checked.
fn successful_stub(name: &str) -> String {
    let dir = std::env::temp_dir()
        .join("bugsleuth-sweep-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");

    #[cfg(windows)]
    let path = {
        let path = dir.join("stub.cmd");
        std::fs::write(
            &path,
            "@echo off\r\nfindstr /R \".*\" > nul\r\necho {\"structured_output\":{\"findings\":[]},\"is_error\":false,\"num_turns\":1}\r\nexit /b 0\r\n",
        )
        .expect("write stub");
        path
    };
    #[cfg(not(windows))]
    let path = {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("stub.sh");
        std::fs::write(
            &path,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"structured_output\":{\"findings\":[]},\"is_error\":false,\"num_turns\":1}'\n",
        )
        .expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };
    path.to_string_lossy().into_owned()
}

/// Both report paths record the resolved label, not the raw config string.
///
/// Replaces a source scan for `let model_label = resolved_label(` in sweep.rs.
/// That assignment stays necessary for anchor verification further down the
/// file, so its presence proved nothing about either `LaneReport` initializer:
/// both could regress to `request.model.to_string()` with the scan still green.
/// A bare `sonnet` would then be compared against a recorded `claude:sonnet`,
/// and a cancelled run would report already-finished sweeps as never reached.
#[tokio::test]
async fn bare_alias_reports_use_the_resolved_label_on_both_paths() {
    for (stub, swept) in [
        (echoing_stub("bare-label-failure"), false),
        (successful_stub("bare-label-success"), true),
    ] {
        let report = run(Request {
            repo: Path::new("."),
            lane: Lane::Correctness,
            model: "sonnet",
            scope: None,
            effort: "",
            max_turns: 1,
            timeout: Duration::from_secs(60),
            api_key: None,
            binary: Some(&stub),
        })
        .await;
        assert_eq!(report.model, "claude:sonnet");
        assert_eq!(
            matches!(&report.status, Status::Swept { .. }),
            swept,
            "the stub did not take the path this case is about: {:?}",
            report.status
        );
    }
}

#[tokio::test]
async fn the_vendor_prefix_never_reaches_the_cli() {
    // 0.2.19 passed the whole spec to `-m`, so every Kilo and Codex sweep in a
    // run was refused with `Model not found: kilo:kilo/kimi-coding/...` before
    // it read a line of code. Claude exercises the shared split without Kilo's
    // worktree/config preflight or the deliberately disabled Codex review.
    let stub = echoing_stub("model-arg");
    let report = run(Request {
        repo: Path::new("."),
        lane: Lane::Correctness,
        model: "claude:sonnet",
        scope: None,
        effort: "",
        max_turns: 1,
        timeout: Duration::from_secs(60),
        api_key: None,
        binary: Some(&stub),
    })
    .await;

    let Status::NotSwept { reason } = &report.status else {
        panic!(
            "a CLI that exits non-zero cannot sweep: {:?}",
            report.status
        );
    };
    // The known-present half. Without it, a reason that never carried the argv
    // at all — an unspawnable stub, an undelivered prompt — would satisfy the
    // assertion below by saying nothing.
    assert!(
        reason.contains("sonnet"),
        "the stub should have echoed the model it was given: {reason}"
    );
    assert!(
        !reason.contains("claude:sonnet"),
        "`vendor:model` is BugSleuth's own notation and no CLI knows it: {reason}"
    );
    assert_eq!(
        report.model, "claude:sonnet",
        "the report still records which vendor was asked"
    );
}

#[tokio::test]
async fn agent_mode_reaches_each_supported_provider_prompt_and_claudes_tool_policy() {
    let stub = echoing_stub("agent-mode");
    let report = run_with_agents(
        Request {
            repo: Path::new("."),
            lane: Lane::Security,
            model: "sonnet",
            scope: None,
            effort: "",
            max_turns: 1,
            timeout: Duration::from_secs(60),
            api_key: None,
            binary: Some(&stub),
        },
        true,
    )
    .await;
    let Status::NotSwept { reason } = report.status else {
        panic!("a failing stub cannot complete a sweep");
    };
    assert!(reason.contains("foreground Explore subagents"), "{reason}");
    assert!(
        reason.contains("--tools Read,Glob,Grep,Agent")
            || reason.contains("--tools \"Read,Glob,Grep,Agent\""),
        "{reason}"
    );
    assert!(reason.contains("--effort ultracode"), "{reason}");
}

#[test]
fn an_unknown_prefix_is_treated_as_a_model_name_not_silently_dropped() {
    // Model ids legitimately contain colons, so an unrecognised prefix must
    // not be swallowed as a vendor.
    assert_eq!(
        Vendor::parse("anthropic:claude-opus-5"),
        (Vendor::Claude, "anthropic:claude-opus-5")
    );
}
