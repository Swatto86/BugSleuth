//! Tests for planning, in their own file only because the module plus its
//! tests crossed the hard line cap.

use super::*;

fn config(entries: &[(&str, &[&str])]) -> Config {
    Config {
        models: entries
            .iter()
            .map(|(id, lanes)| ModelPlan {
                id: (*id).to_string(),
                lanes: lanes.iter().map(|l| (*l).to_string()).collect(),
                effort: String::new(),
                use_agents: false,
                passes: 1,
            })
            .collect(),
    }
}

fn with_effort(id: &str, effort: &str) -> Config {
    Config {
        models: vec![ModelPlan {
            id: id.to_string(),
            lanes: vec!["correctness".to_string()],
            effort: effort.to_string(),
            use_agents: false,
            passes: 1,
        }],
    }
}

#[test]
fn an_effort_the_vendor_does_not_accept_is_refused_before_anything_is_paid_for() {
    // A typo is otherwise discovered by a CLI rejecting the invocation
    // after the sweep was queued - or worse, by it being ignored, so the
    // run completes and nothing says the depth asked for was never used.
    let refused = plan(&with_effort("sonnet", "hihg"));
    let message = refused.map(|_| ()).unwrap_err().to_string();
    assert!(message.contains("hihg"), "{message}");
    assert!(message.contains("low, medium, high"), "{message}");
}

#[test]
fn claude_effort_is_validated_per_model() {
    for level in ["low", "medium", "high", "max"] {
        assert!(
            plan(&with_effort("sonnet", level)).is_ok(),
            "{level} refused"
        );
    }
    assert!(plan(&with_effort("opus", "xhigh")).is_ok());
    assert!(plan(&with_effort("sonnet", "xhigh")).is_ok());
    assert!(plan(&with_effort("haiku", "high")).is_err());
    assert!(plan(&with_effort("sonnet", "")).is_ok());
    assert!(plan(&with_effort("sonnet", "  ")).is_ok());
}

#[test]
fn a_vendor_whose_levels_are_discovered_at_runtime_is_not_second_guessed() {
    // Kilo passes --variant through to whichever provider is behind the
    // model, so its accepted values belong to that model. Refusing what
    // cannot be enumerated would block valid configurations.
    assert!(plan(&with_effort("kilo:some/model", "thinking")).is_ok());
    assert!(plan(&with_effort("kilo:some/model", "anything-at-all")).is_ok());
}

#[test]
fn kilo_cannot_request_agents_its_read_only_ask_agent_cannot_delegate_to() {
    let config: Config = serde_json::from_str(
        r#"{"models":[{"id":"kilo:anthropic/claude-sonnet-5","lanes":["security"],"effort":"thinking","use_agents":true}]}"#,
    )
    .expect("config parses");
    let message = plan(&config).map(|_| ()).unwrap_err().to_string();
    assert!(message.contains("cannot delegate"), "{message}");
}

#[test]
fn claude_ultracode_replaces_a_separate_effort_setting() {
    let mut config = with_effort("fable", "high");
    config.models[0].use_agents = true;
    let message = plan(&config).map(|_| ()).unwrap_err().to_string();
    assert!(message.contains("leave effort at its default"), "{message}");

    let mut default_model = with_effort("claude:", "high");
    default_model.models[0].use_agents = true;
    let message = plan(&default_model).map(|_| ()).unwrap_err().to_string();
    assert!(message.contains("leave effort at its default"), "{message}");
}

#[test]
fn a_huge_pass_count_is_refused_rather_than_enumerated_forever() {
    // The backend must not trust the UI's picker to bound this: settings
    // are a JSON file a person can edit. Before the bound, a large value
    // was enumerated into millions of units and the run never started.
    let mut config = config(&[("sonnet", &["correctness"])]);
    config.models[0].passes = 26; // one over the cap, so the test runs instantly
    assert!(
        plan(&config).is_err(),
        "a pass count above the cap must be refused"
    );
}

#[test]
fn a_lane_with_no_model_is_reported_rather_than_quietly_omitted() {
    let plan = plan(&config(&[("sonnet", &["correctness"])]))
        .unwrap_or_else(|e| panic!("plan failed: {e}"));
    assert_eq!(plan.units.len(), 1);
    assert_eq!(plan.uncovered.len(), Lane::ALL.len() - 1);
    assert!(plan.uncovered.contains(&Lane::Security));
}

#[test]
fn a_fully_covered_run_reports_no_gaps() {
    // Derived from `Lane::ALL`, not written out: a hand-copied lane list
    // means adding a lane fails this test for no defect, and the natural
    // repair is to paste the new name in without asking whether the rest of
    // the product covers it.
    let every: Vec<&str> = Lane::ALL.iter().map(|lane| lane.slug()).collect();
    let plan = plan(&config(&[("sonnet", &every)])).unwrap_or_else(|e| panic!("plan failed: {e}"));
    assert!(plan.uncovered.is_empty());
    assert_eq!(plan.units.len(), Lane::ALL.len());
}

#[test]
fn plan_refuses_codex_repository_review() {
    let refused = plan(&with_effort("codex:gpt-5.6-codex", ""));
    let message = refused.map(|_| ()).unwrap_err().to_string();
    assert!(
        message.contains("repository review"),
        "must name the disabled capability: {message}"
    );
    assert!(
        message.contains("disabled") || message.contains("not repository-bounded"),
        "must say why Codex cannot review: {message}"
    );
}

#[test]
fn two_models_on_one_lane_is_two_units_because_that_is_the_point() {
    let plan = plan(&config(&[
        ("sonnet", &["correctness"]),
        ("kilo:", &["correctness"]),
    ]))
    .unwrap_or_else(|e| panic!("plan failed: {e}"));
    assert_eq!(plan.units.len(), 2);
}

#[test]
fn the_same_model_assigned_a_lane_twice_is_not_paid_for_twice() {
    let plan = plan(&config(&[("sonnet", &["correctness", "correctness"])]))
        .unwrap_or_else(|e| panic!("plan failed: {e}"));
    assert_eq!(plan.units.len(), 1);
}

#[test]
fn a_configuration_that_would_run_nothing_is_an_error_not_an_empty_report() {
    assert!(plan(&config(&[])).is_err());
    assert!(plan(&config(&[("sonnet", &[])])).is_err());
}

#[test]
fn an_unknown_lane_name_is_rejected_rather_than_silently_skipped() {
    assert!(plan(&config(&[("sonnet", &["correctnes"])])).is_err());
}

#[test]
fn every_provider_runs_at_most_one_sweep_per_batch() {
    // No provider publishes a safe maximum for independent authenticated CLI
    // processes, and two real Kilo processes collided in its credential store.
    // Different providers may overlap, but one provider must stay serial.
    let plan = plan(&config(&[
        ("sonnet", &["security", "ux"]),
        ("kimi:model", &["security", "ux"]),
        ("kilo:model", &["security", "ux"]),
    ]))
    .unwrap_or_else(|e| panic!("plan failed: {e}"));

    for batch in plan.batches() {
        let vendors: Vec<String> = batch.iter().map(|unit| vendor_of(&unit.model)).collect();
        let unique: BTreeSet<&str> = vendors.iter().map(String::as_str).collect();
        assert_eq!(vendors.len(), unique.len(), "provider overlap: {batch:?}");
    }
}

#[test]
fn batching_runs_every_unit_exactly_once() {
    let plan = plan(&config(&[
        ("sonnet", &["correctness", "security", "ux"]),
        ("kilo:", &["correctness"]),
    ]))
    .unwrap_or_else(|e| panic!("plan failed: {e}"));

    let scheduled: usize = plan.batches().iter().map(Vec::len).sum();
    assert_eq!(scheduled, plan.units.len());
}

#[test]
fn a_bare_model_name_is_treated_as_the_claude_vendor_for_batching() {
    assert_eq!(vendor_of("sonnet"), "claude");
    assert_eq!(vendor_of("claude:opus"), "claude");
    assert_eq!(vendor_of("codex:gpt"), "codex");
    assert_eq!(vendor_of("kilo:"), "kilo");
    // A model id that merely contains a colon is not a vendor prefix.
    assert_eq!(vendor_of("anthropic:claude-opus-5"), "claude");
}

#[test]
fn equivalent_claude_spellings_collapse_to_one_unit() {
    // `sonnet` and `claude:sonnet` invoke the same model; keying units on the
    // raw string scheduled and charged them as two.
    let plan = plan(&config(&[
        ("sonnet", &["correctness"]),
        ("claude:sonnet", &["correctness"]),
    ]))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(plan.units.len(), 1, "{:?}", plan.units);
    assert_eq!(plan.units[0].model, "sonnet");
}

#[test]
fn canonical_spec_collapses_claude_but_keeps_the_default_and_other_vendors() {
    assert_eq!(canonical_spec("sonnet"), "sonnet");
    assert_eq!(canonical_spec("claude:sonnet"), "sonnet");
    assert_eq!(canonical_spec(" claude:sonnet "), "sonnet");
    // The bare default is not a specific model, so it is kept as-is.
    assert_eq!(canonical_spec("claude:"), "claude:");
    assert_eq!(canonical_spec("codex:gpt-5.6"), "codex:gpt-5.6");
    assert_eq!(canonical_spec("kilo:z-ai/glm"), "kilo:z-ai/glm");
}

/// Every vendor prefix must be recognised here, or a model is filed under the
/// wrong one.
///
/// `vendor_of` decides which effort rules apply and which sweeps may run
/// concurrently. An unrecognised prefix silently answers "claude", so a Kimi
/// model was checked against Claude's effort rules — which accept anything
/// unlisted — and an effort Kimi has nowhere to put was waved through instead
/// of refused. The CLI then ignored it and the report never said the depth
/// asked for was not applied.
#[test]
fn every_vendor_prefix_is_recognised() {
    assert_eq!(super::vendor_of("sonnet"), "claude");
    assert_eq!(super::vendor_of("claude:opus"), "claude");
    assert_eq!(super::vendor_of("codex:gpt-5.6-codex"), "codex");
    assert_eq!(super::vendor_of("kilo:kilo/x"), "kilo");
    assert_eq!(
        super::vendor_of("kimi:kimi-code/k3"),
        "kimi",
        "a Kimi model was filed under another vendor's rules"
    );
    assert_eq!(
        super::canonical_spec("kimi:kimi-code/k3"),
        "kimi:kimi-code/k3"
    );
}

/// And the effort refusal actually reaches a Kimi model.
#[test]
fn an_effort_is_refused_for_a_vendor_that_has_no_such_flag() {
    let error = super::check_effort("kimi:kimi-code/k3", "high")
        .expect_err("Kimi has no effort flag, so an effort must not be accepted");
    assert!(error.to_string().contains("kimi"), "{error}");
    assert!(super::check_effort("kimi:kimi-code/k3", "").is_ok());
}

#[test]
fn cursor_vendor_and_effort_rules() {
    assert_eq!(super::vendor_of("cursor:composer-2.5"), "cursor");
    assert_eq!(
        super::canonical_spec("cursor:composer-2.5"),
        "cursor:composer-2.5"
    );
    let error = super::check_effort("cursor:composer-2.5", "high").expect_err("no effort flag");
    assert!(error.to_string().contains("cursor"), "{error}");
    assert!(super::check_effort("cursor:composer-2.5", "").is_ok());
}
