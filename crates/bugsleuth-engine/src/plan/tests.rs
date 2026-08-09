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
    assert!(plan(&with_effort("sonnet", "xhigh")).is_err());
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
fn two_models_on_one_lane_is_two_units_because_that_is_the_point() {
    let plan = plan(&config(&[
        ("sonnet", &["correctness"]),
        ("codex:", &["correctness"]),
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
fn at_concurrency_one_no_two_sweeps_of_the_same_vendor_share_a_batch() {
    // Concurrency of one is the old strictly-sequential-per-vendor behaviour,
    // and it must still be reachable: it is the guard someone dials to when a
    // vendor's rate limit bites.
    let plan = plan(&config(&[
        ("sonnet", &["correctness", "security"]),
        ("claude:opus", &["contract"]),
        ("codex:", &["correctness"]),
        ("kilo:", &["ux"]),
    ]))
    .unwrap_or_else(|e| panic!("plan failed: {e}"));

    for batch in plan.batches(1) {
        let mut vendors: Vec<String> = batch.iter().map(|u| vendor_of(&u.model)).collect();
        let before = vendors.len();
        vendors.sort();
        vendors.dedup();
        assert_eq!(
            before,
            vendors.len(),
            "a batch ran one vendor twice at once"
        );
    }
}

#[test]
fn a_higher_concurrency_lets_several_sweeps_of_one_vendor_share_a_batch() {
    // The whole point of the setting: three Claude models on one lane should be
    // able to run at once, so five Claude sweeps at concurrency three take two
    // rounds rather than five. No batch may exceed the limit for any vendor.
    let plan = plan(&config(&[(
        "sonnet",
        &["correctness", "security", "contract", "ux", "gate"],
    )]))
    .unwrap_or_else(|e| panic!("plan failed: {e}"));
    assert_eq!(plan.units.len(), 5);

    let batches = plan.batches(3);
    assert_eq!(
        batches.len(),
        2,
        "five sweeps at three-per-round is two rounds"
    );
    for batch in &batches {
        let mut per_vendor = std::collections::BTreeMap::<String, usize>::new();
        for unit in batch {
            *per_vendor.entry(vendor_of(&unit.model)).or_insert(0) += 1;
        }
        assert!(
            per_vendor.values().all(|&n| n <= 3),
            "a batch ran a vendor more than the limit"
        );
    }
    let scheduled: usize = batches.iter().map(Vec::len).sum();
    assert_eq!(
        scheduled,
        plan.units.len(),
        "a unit was dropped or duplicated"
    );
}

#[test]
fn a_zero_concurrency_is_treated_as_one_and_does_not_loop_forever() {
    // A hand-edited settings file could carry zero; taking no units per batch
    // would spin the loop forever. It must behave exactly like one.
    let plan = plan(&config(&[("sonnet", &["correctness", "security"])]))
        .unwrap_or_else(|e| panic!("plan failed: {e}"));
    assert_eq!(plan.batches(0).len(), plan.batches(1).len());
}

#[test]
fn a_vendors_concurrent_slots_go_to_different_models_before_doubling_up() {
    // The regression a user caught: two Codex models on five lanes each, at two
    // per round, spent both slots on the FIRST model's lanes — so the second
    // model they chose did not start until the first was nearly done. Picking
    // two models means wanting them run together; the first round must contain
    // both models, not one model twice.
    let plan = plan(&config(&[
        (
            "codex:gpt-a",
            &["correctness", "security", "contract", "ux", "gate"],
        ),
        (
            "codex:gpt-b",
            &["correctness", "security", "contract", "ux", "gate"],
        ),
    ]))
    .unwrap_or_else(|e| panic!("plan failed: {e}"));

    let first = &plan.batches(2)[0];
    let models: BTreeSet<&str> = first.iter().map(|u| u.model.as_str()).collect();
    assert_eq!(
        models,
        BTreeSet::from(["codex:gpt-a", "codex:gpt-b"]),
        "the first round ran one model twice instead of both models: {first:?}"
    );
}

#[test]
fn one_models_extra_passes_still_fill_the_concurrency_when_it_is_alone() {
    // The diversity rule must not starve the single-model case: two passes of
    // one model with room for two should still run at once, or asking for more
    // passes would silently serialise them.
    let two_passes = Config {
        models: vec![ModelPlan {
            id: "sonnet".to_string(),
            lanes: vec!["correctness".to_string()],
            effort: String::new(),
            passes: 2,
        }],
    };
    let plan = plan(&two_passes).unwrap_or_else(|e| panic!("plan failed: {e}"));
    assert_eq!(plan.units.len(), 2, "two passes should be two units");
    let batches = plan.batches(2);
    assert_eq!(batches.len(), 1, "both passes should share one round");
    assert_eq!(batches[0].len(), 2);
}

#[test]
fn batching_runs_every_unit_exactly_once() {
    let plan = plan(&config(&[
        ("sonnet", &["correctness", "security", "ux"]),
        ("codex:", &["correctness"]),
    ]))
    .unwrap_or_else(|e| panic!("plan failed: {e}"));

    // The count is invariant to how far the fan-out goes: every unit runs once
    // whether the vendor is serialised or fully parallel.
    for concurrency in [1, 2, 5] {
        let scheduled: usize = plan.batches(concurrency).iter().map(Vec::len).sum();
        assert_eq!(scheduled, plan.units.len(), "at concurrency {concurrency}");
    }
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
