use super::*;

#[test]
fn a_bare_model_name_means_claude_so_the_common_case_stays_short() {
    assert_eq!(Vendor::parse("sonnet"), (Vendor::Claude, "sonnet"));
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

#[tokio::test]
async fn a_kilo_sweep_stops_at_the_preflight_before_doing_any_work() {
    // Whichever way this machine is configured, a Kilo sweep must consult
    // the preflight before discovering a provider or building a worktree.
    // The two outcomes are asserted against each other rather than against
    // a fixed expectation, because the honest answer depends on the config:
    // a refusal must name the open tool, and a pass must mean the config
    // really denies the network. Both halves of `network_gap` are tested
    // directly, against written configs, in the provider crate.
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

    match (kilo::preflight::network_gap(), report.status) {
        (Some(_), Status::NotSwept { reason }) => assert!(
            reason.contains("webfetch") || reason.contains("websearch"),
            "a refusal must name the tool left open: {reason}"
        ),
        (Some(gap), other) => {
            panic!("network is open ({gap}) but the sweep was not refused: {other:?}")
        }
        // Network denied: the preflight is satisfied and the sweep proceeds
        // to the next failure, which without a Kilo binary is a real one.
        (None, _) => {}
    }
}

/// A stand-in CLI that fails, printing on stderr the argv it was handed.
///
/// The argv is the only place the answer lives: a test that built a vendor spec
/// itself would be asserting against a copy of the wiring, and the wiring is
/// exactly what broke. Stdin is drained before exiting, because a CLI that dies
/// without reading the brief is reported as an undelivered prompt rather than as
/// a failed invocation, and the argv would never reach the report.
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
            "@echo off\r\necho %* 1>&2\r\nmore > nul\r\nexit /b 1\r\n",
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
            "#!/bin/sh\necho \"$@\" >&2\ncat >/dev/null\nexit 1\n",
        )
        .expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn the_vendor_prefix_never_reaches_the_cli() {
    // 0.2.19 passed the whole spec to `-m`, so every Kilo and Codex sweep in a
    // run was refused with `Model not found: kilo:kilo/kimi-coding/...` before
    // it read a line of code. Codex is the arm exercised here because it needs
    // neither a worktree nor any particular machine config; all three arms are
    // handed the same variable.
    let stub = echoing_stub("model-arg");
    let report = run(Request {
        repo: Path::new("."),
        lane: Lane::Correctness,
        model: "codex:gpt-5.6-codex",
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
        reason.contains("gpt-5.6-codex"),
        "the stub should have echoed the model it was given: {reason}"
    );
    assert!(
        !reason.contains("codex:gpt-5.6-codex"),
        "`vendor:model` is BugSleuth's own notation and no CLI knows it: {reason}"
    );
    assert_eq!(
        report.model, "codex:gpt-5.6-codex",
        "the report still records which vendor was asked"
    );
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
