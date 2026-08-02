//! Tests for the orchestrator, in their own file only because the
//! orchestrator plus its tests crossed the hard line cap.

use super::super::orchestrate::persist::file_name_for;
use super::super::orchestrate::*;
use super::super::plan::{Config, ModelPlan, Unit, plan};
use bugsleuth_domain::Lane;
use std::path::Path;
use std::time::Duration;

/// The whole orchestration path, end to end, with no model involved.
///
/// Every unit is pre-seeded as a completed sweep and resumed, so this
/// exercises planning, reuse, merging and reporting for real while costing
/// nothing. Without it, the only proof `run` works would be having watched
/// it once.
#[tokio::test]
async fn a_fully_resumed_run_merges_previous_sweeps_without_calling_any_model() {
    use crate::plan::{Config, ModelPlan};

    let dir = std::env::temp_dir()
        .join("bugsleuth-run-tests")
        .join(format!("{}-resumed", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // Two vendors reporting the same defect in different words.
    let seed = |model: &str, title: &str, explanation: &str| {
        let report = format!(
            r#"{{"lane":"Correctness","model":"{model}","status":{{"state":"swept"}},
                    "findings":[{{"id":"x","lane":"correctness","model":"{model}",
                      "title":"{title}","severity":"high",
                      "anchor":{{"file":"src/a.rs","line":10,"claimed_line":10,"snippet":"code"}},
                      "explanation":"{explanation}","failure_scenario":"f"}}],
                    "rejected":[]}}"#
        );
        let unit = Unit {
            model: model.to_string(),
            lane: Lane::Correctness,
            effort: String::new(),
            pass: 1,
        };
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(file_name_for(&unit)), report);
    };
    seed(
        "claude:sonnet",
        "average_price divides by zero on an empty inventory",
        "No check for an empty inventory before dividing by the item count.",
    );
    seed(
        "codex:",
        "Calculating the average price of an empty inventory panics",
        "An empty inventory has length zero so this integer division panics.",
    );

    let plan = crate::plan::plan(&Config {
        models: vec![
            ModelPlan {
                id: "claude:sonnet".into(),
                lanes: vec!["correctness".into()],
                effort: String::new(),
                passes: 1,
            },
            ModelPlan {
                id: "codex:".into(),
                lanes: vec!["correctness".into()],
                effort: String::new(),
                passes: 1,
            },
        ],
    })
    .unwrap_or_else(|e| panic!("plan failed: {e}"));

    let report = run(
        &plan,
        RunOptions {
            repo: Path::new("."),
            scope: None,
            triage_model: "",
            cancel: Default::default(),
            max_turns: 1,
            timeout: Duration::from_secs(1),
            api_key: None,
            out_dir: Some(&dir),
            resume: true,
            progress: None,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("run failed: {e}"));

    assert_eq!(report.swept.len(), 2, "both sweeps should have been reused");
    assert_eq!(
        report.ranked.len(),
        1,
        "the same defect from two vendors should merge into one"
    );
    assert_eq!(report.ranked[0].cluster.agreement, 2);

    // Three lanes had no model, and must be visible as holes.
    assert_eq!(report.gaps.len(), 3);
    let text = report.to_text();
    assert!(text.contains("NOT SWEPT"));
    assert!(text.contains("found by 2 of 2 models"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_cancelled_run_names_every_sweep_it_never_reached() {
    // The report's whole discipline is that an absent finding is never silent.
    // A run stopped after one of four sweeps must not look like a run that
    // covered everything and found little.
    let plan = plan(&Config {
        models: vec![ModelPlan {
            id: "no-such-model-please".to_string(),
            lanes: vec!["correctness".to_string(), "security".to_string()],
            effort: String::new(),
            passes: 1,
        }],
    })
    .unwrap_or_else(|e| panic!("plan failed: {e}"));

    let cancel = crate::cancel::Cancel::new();
    cancel.stop();

    let report = run(
        &plan,
        RunOptions {
            repo: Path::new("."),
            scope: None,
            triage_model: "",
            cancel: cancel.clone(),
            max_turns: 1,
            timeout: Duration::from_secs(1),
            api_key: None,
            out_dir: None,
            resume: false,
            progress: None,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("run failed: {e}"));

    let cancelled: Vec<&Gap> = report
        .gaps
        .iter()
        .filter(|gap| gap.reason.contains("cancelled"))
        .collect();
    assert_eq!(cancelled.len(), 2, "sweeps went missing rather than named");
    assert!(report.to_text().contains("NOT SWEPT"));
}
