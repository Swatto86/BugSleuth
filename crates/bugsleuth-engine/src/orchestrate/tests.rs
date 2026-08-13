//! Tests for the orchestrator, in their own file only because the
//! orchestrator plus its tests crossed the hard line cap.

use super::super::orchestrate::persist::{file_name_for, write_report};
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

    // Resume now requires a report to name the clean revision it reviewed, so
    // the run happens against a real clean checkout and every seeded sweep
    // records that revision. Without it, resume would sweep again — and there
    // is no model here to sweep with.
    let (repo, rev) = clean_checkout(&dir);

    // Two vendors reporting the same defect in different words.
    let seed = |model: &str, title: &str, explanation: &str, usage: &str| {
        let report = format!(
            r#"{{"lane":"Correctness","model":"{model}","commit":"{rev}","cache_revision":"{rev}","usage":"{usage}","status":{{"state":"swept"}},
                    "findings":[{{"id":"x","lane":"correctness","model":"{model}",
                      "title":"{title}","severity":"high",
                      "anchor":{{"file":"src/a.rs","line":10,"claimed_line":10,"snippet":"code"}},
                      "explanation":"{explanation}","failure_scenario":"f"}}],
                    "rejected":[]}}"#
        );
        // A real run writes each report under the canonical unit name the plan
        // produces, so the seed must too — `claude:sonnet` is filed as `sonnet`.
        let unit = Unit {
            model: crate::plan::canonical_spec(model),
            lane: Lane::Correctness,
            effort: String::new(),
            use_agents: false,
            pass: 1,
        };
        let report = serde_json::from_str(&report)
            .unwrap_or_else(|error| panic!("invalid seeded report: {error}"));
        assert!(write_report(&dir, &file_name_for(&unit), &report).is_ok());
    };
    seed(
        "claude:sonnet",
        "average_price divides by zero on an empty inventory",
        "No check for an empty inventory before dividing by the item count.",
        "input_tokens=120 output_tokens=12",
    );
    seed(
        "kilo:",
        "Calculating the average price of an empty inventory panics",
        "An empty inventory has length zero so this integer division panics.",
        "input_tokens=80 output_tokens=8",
    );

    let plan = crate::plan::plan(&Config {
        models: vec![
            ModelPlan {
                id: "claude:sonnet".into(),
                lanes: vec!["correctness".into()],
                effort: String::new(),
                use_agents: false,
                passes: 1,
            },
            ModelPlan {
                id: "kilo:".into(),
                lanes: vec!["correctness".into()],
                effort: String::new(),
                use_agents: false,
                passes: 1,
            },
        ],
    })
    .unwrap_or_else(|e| panic!("plan failed: {e}"));

    let report = run(
        &plan,
        RunOptions {
            repo: &repo,
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
    assert!(report.swept.iter().all(|sweep| {
        sweep.commit.as_deref() == Some(&rev)
            && sweep.cache_revision.as_deref() == Some(&rev)
            && sweep.scope.is_none()
            && sweep
                .usage
                .as_deref()
                .is_some_and(|usage| usage.contains("input_tokens="))
    }));

    // Only Correctness had a model. Every other lane must be visible as a
    // hole — counted from `Lane::ALL` rather than written out, so a lane added
    // later is one this assertion already covers.
    assert_eq!(report.gaps.len(), Lane::ALL.len() - 1);
    let text = report.to_text();
    assert!(text.contains("NOT SWEPT"));
    assert!(text.contains("found by 2 of 2 models"));
    assert!(text.contains("input_tokens=120"), "{text}");
    assert!(text.contains("input_tokens=80"), "{text}");
    assert!(text.contains(&rev[..7]), "{text}");
    assert!(text.contains("pinned"), "{text}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn live_sweep_metadata_reaches_the_aggregate_report() {
    use crate::report::{LaneReport, Status};

    let lane_report = LaneReport {
        lane: "Security".into(),
        model: "claude:sonnet".into(),
        commit: Some("1234567890abcdef".into()),
        cache_revision: None,
        scope: Some("src/security".into()),
        excluded_paths: vec![],
        status: Status::Swept {
            turns: Some(4),
            salvaged: false,
        },
        findings: vec![],
        rejected: vec![],
        usage: Some("input_tokens=42 output_tokens=7".into()),
    };
    let swept = Swept::from_report(Lane::Security, &lane_report);
    assert_eq!(swept.commit, lane_report.commit);
    assert_eq!(swept.cache_revision, lane_report.cache_revision);
    assert_eq!(swept.scope, lane_report.scope);
    assert_eq!(swept.usage, lane_report.usage);

    let text = RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept: vec![swept],
        gaps: vec![],
        cancelled: false,
    }
    .to_text();
    assert!(text.contains("scope: src/security"), "{text}");
    assert!(text.contains("revision 1234567, unpinned"), "{text}");
    assert!(text.contains("input_tokens=42"), "{text}");
    assert!(
        text.contains("consistency across this run is unconfirmed"),
        "{text}"
    );
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
            use_agents: false,
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

#[test]
fn a_sweep_whose_task_died_is_reported_as_a_gap_not_omitted() {
    // The comment beside this code demanded it for weeks while the code only
    // printed a warning, so the unit vanished from the report - which reads
    // exactly like a lane that ran and found nothing. Found by this tool
    // reviewing itself.
    let report = RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept: vec![],
        gaps: vec![Gap {
            lane: Lane::Correctness,
            model: None,
            reason: "a sweep failed to complete and produced nothing: task panicked".into(),
        }],
        cancelled: false,
    };
    let text = report.to_text();
    assert!(text.contains("NOT SWEPT"), "{text}");
    assert!(text.contains("produced nothing"), "{text}");
}

/// A model spelled as a bare alias must compare equal to the label a report
/// records, or a cancelled run counts finished sweeps as still outstanding.
///
/// The defect: `remaining_units.retain` compared `unit.model` — the raw config
/// string, "sonnet" — against `report.model`, the resolved "claude:sonnet".
/// That is never equal, so nothing was ever removed and the cancellation
/// summary told the reader that lanes it had already swept were not reached.
#[test]
fn a_bare_alias_resolves_to_the_label_a_report_records() {
    use crate::sweep::resolved_label;
    assert_eq!(resolved_label("sonnet"), "claude:sonnet");
    assert_eq!(resolved_label("claude:sonnet"), "claude:sonnet");
    assert_eq!(resolved_label("codex:"), "codex:");
    assert_eq!(resolved_label("kilo:kimi"), "kilo:kimi");
}

// The scan that used to live here looked for `let model_label = resolved_label(`
// in sweep.rs. That assignment is still needed for anchor verification further
// down the file, so its presence said nothing about either `LaneReport`
// initializer: both could regress to the raw request model with the scan green.
// Replaced by sweep::tests::bare_alias_reports_use_the_resolved_label_on_both_paths,
// which drives the real sweep boundary with a bare alias on both paths.

/// One finished sweep accounts for exactly one outstanding unit.
///
/// The defect: the cancellation bookkeeping removed every unit matching the
/// finished sweep's lane and model. A model configured for three passes had all
/// three struck off when the first finished, so a run cancelled after pass one
/// reported the other two as accounted for — a claim of coverage that never
/// happened, in the summary whose whole job is saying what did not run.
///
/// Calls the real `strike_off`. The first version of this test re-implemented
/// the predicate, which would have passed happily while the code it was meant
/// to protect regressed.
#[test]
fn finishing_one_pass_leaves_the_other_passes_outstanding() {
    use crate::plan::Unit;
    use bugsleuth_domain::Lane;

    let unit = |pass: usize| Unit {
        model: "sonnet".into(),
        lane: Lane::Correctness,
        pass,
        effort: String::new(),
        use_agents: false,
    };
    let mut remaining = vec![unit(1), unit(2), unit(3)];

    strike_off(&mut remaining, Lane::Correctness, "claude:sonnet");
    assert_eq!(
        remaining.len(),
        2,
        "one sweep struck off more than one unit"
    );

    strike_off(&mut remaining, Lane::Correctness, "claude:sonnet");
    strike_off(&mut remaining, Lane::Correctness, "claude:sonnet");
    assert!(
        remaining.is_empty(),
        "three sweeps did not account for three units"
    );

    // A sweep of something else accounts for nothing here.
    let mut other = vec![unit(1)];
    strike_off(&mut other, Lane::Security, "claude:sonnet");
    strike_off(&mut other, Lane::Correctness, "codex:");
    assert_eq!(other.len(), 1, "an unrelated sweep struck off a unit");
}

/// A clean git checkout with one commit, and its HEAD — for resume tests that
/// need a report's recorded revision to match the repository.
fn clean_checkout(parent: &std::path::Path) -> (std::path::PathBuf, String) {
    let repo = parent.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .expect("git runs")
                .status
                .success(),
            "git {args:?} failed"
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "Test"]);
    std::fs::write(repo.join("seed.txt"), "seed\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "seed"]);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .expect("git runs");
    let rev = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (repo, rev)
}
