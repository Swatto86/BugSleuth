//! Tests for the report renderer, in their own file only because the
//! renderer plus its tests crossed the hard line cap.

use super::super::*;
use crate::orchestrate::{Gap, Swept};

fn report(gaps: Vec<Gap>) -> RunReport {
    RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept: vec![Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Correctness,
            commit: Some("aaaaaaaa".into()),
            cache_revision: Some("aaaaaaaa".into()),
            scope: None,
            usage: None,
            findings: 0,
            rejected: 0,
            salvaged: false,
        }],
        gaps,
        cancelled: false,
    }
}

#[test]
fn a_lane_with_no_model_is_named_in_the_report() {
    let text = report(vec![Gap {
        lane: Lane::Security,
        model: None,
        reason: "no model is assigned to this lane".into(),
    }])
    .to_text();
    assert!(text.contains("NOT SWEPT"));
    assert!(text.contains("Security"));
    assert!(text.contains("NOT reviewed"));
}

#[test]
fn a_failed_sweep_is_named_with_its_reason() {
    let text = report(vec![Gap {
        lane: Lane::Ux,
        model: Some("kilo:".into()),
        reason: "the kilo CLI exited with code 1".into(),
    }])
    .to_text();
    assert!(text.contains("NOT SWEPT"));
    assert!(text.contains("kilo:"));
    assert!(text.contains("exited with code 1"));
}

#[test]
fn a_single_lane_report_does_not_warn_about_comparing_lanes() {
    assert!(!report(vec![]).to_text().contains("NOT directly comparable"));
}

#[test]
fn a_multi_lane_report_warns_that_severities_are_not_comparable() {
    // A "high" from the security lane and a "high" from the correctness lane
    // were assigned by models answering different questions.
    let multi = RunReport {
        ranked: vec![],
        swept: vec![
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Correctness,
                commit: Some("aaaaaaaa".into()),
                cache_revision: Some("aaaaaaaa".into()),
                scope: None,
                usage: None,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Security,
                commit: Some("aaaaaaaa".into()),
                cache_revision: Some("aaaaaaaa".into()),
                scope: None,
                usage: None,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
        ],
        triage: Default::default(),
        gaps: vec![],
        cancelled: false,
    };
    assert_eq!(multi.lanes_swept(), 2);
    assert!(multi.to_text().contains("NOT directly comparable"));
}

#[test]
fn two_models_on_one_lane_is_still_one_lane() {
    let same_lane = RunReport {
        ranked: vec![],
        swept: vec![
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Correctness,
                commit: Some("aaaaaaaa".into()),
                cache_revision: Some("aaaaaaaa".into()),
                scope: None,
                usage: None,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
            Swept {
                model: "codex:".into(),
                lane: Lane::Correctness,
                commit: Some("aaaaaaaa".into()),
                cache_revision: Some("aaaaaaaa".into()),
                scope: None,
                usage: None,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
        ],
        triage: Default::default(),
        gaps: vec![],
        cancelled: false,
    };
    assert_eq!(same_lane.lanes_swept(), 1);
    assert!(!same_lane.to_text().contains("NOT directly comparable"));
}

#[test]
fn a_complete_run_says_nothing_about_unswept_lanes() {
    let text = report(vec![]).to_text();
    assert!(!text.contains("NOT SWEPT"));
    assert!(!text.contains("NOT reviewed"));
}

#[test]
fn a_swept_lane_that_found_nothing_is_not_confused_with_a_gap() {
    let text = report(vec![]).to_text();
    assert!(text.contains("swept: Correctness lane"));
    assert!(text.contains("0 findings"));
}

#[test]
fn a_report_is_split_into_scanable_sections_with_a_summary() {
    let text = report(vec![]).to_text();
    for heading in [
        "BUGSLEUTH RUN REPORT",
        "REVIEW COVERAGE",
        "SUMMARY",
        "LIMITS",
        "ACTIONABLE FINDINGS",
    ] {
        assert!(text.contains(heading), "missing {heading}:\n{text}");
    }
    assert!(text.contains("Sweeps completed: 1"), "{text}");
    assert!(text.contains("Distinct defects: 0"), "{text}");
    assert!(
        text.contains("Critical: 0 | High: 0 | Medium: 0 | Low: 0"),
        "{text}"
    );
    assert!(
        text.contains("No actionable defects survived verification."),
        "{text}"
    );
}

use crate::triage::Outcome;

fn graded_report(triage: Outcome) -> RunReport {
    RunReport {
        ranked: vec![],
        triage,
        swept: vec![Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Correctness,
            commit: Some("aaaaaaaa".into()),
            cache_revision: Some("aaaaaaaa".into()),
            scope: None,
            usage: None,
            findings: 0,
            rejected: 0,
            salvaged: false,
        }],
        gaps: vec![],
        cancelled: false,
    }
}

#[test]
fn a_report_says_when_its_severities_were_never_graded_together() {
    // The order of the whole report rests on severity. A reader who cannot
    // check the code has to be told whose judgement produced that order.
    let text = graded_report(Outcome {
            note: "severities are each model's own assessment of its own finding, ungraded:                    the triage pass failed (claude CLI not found)"
                .into(),
            ..Default::default()
        })
        .to_text();
    assert!(text.contains("ungraded"), "{text}");
}

#[path = "annotations.rs"]
mod annotations;

#[path = "metadata_tests.rs"]
mod metadata_tests;
