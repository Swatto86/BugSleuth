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

#[test]
fn an_unknown_prefix_is_treated_as_a_model_name_not_silently_dropped() {
    // Model ids legitimately contain colons, so an unrecognised prefix must
    // not be swallowed as a vendor.
    assert_eq!(
        Vendor::parse("anthropic:claude-opus-5"),
        (Vendor::Claude, "anthropic:claude-opus-5")
    );
}
