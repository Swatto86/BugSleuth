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
            findings: 0,
            rejected: 0,
            salvaged: false,
        }],
        gaps,
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
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Security,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
        ],
        triage: Default::default(),
        gaps: vec![],
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
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
            Swept {
                model: "codex:".into(),
                lane: Lane::Correctness,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
        ],
        triage: Default::default(),
        gaps: vec![],
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

use crate::triage::Outcome;

fn graded_report(triage: Outcome) -> RunReport {
    RunReport {
        ranked: vec![],
        triage,
        swept: vec![Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Correctness,
            findings: 0,
            rejected: 0,
            salvaged: false,
        }],
        gaps: vec![],
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

#[test]
fn a_fully_graded_multi_lane_report_stops_warning_that_lanes_cannot_be_compared() {
    // The warning exists because each lane's severities were assigned by
    // models answering different questions. Once one grader has compared the
    // whole list against one rubric, that is no longer what happened, and
    // repeating it would tell the reader not to trust the very step taken to
    // make the ordering trustworthy.
    let mut multi = RunReport {
        ranked: vec![],
        triage: Outcome {
            graded: 0,
            changed: 0,
            note: String::new(),
        },
        swept: vec![
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Correctness,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Security,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
        ],
        gaps: vec![],
    };
    // Nothing graded: the warning stands.
    assert!(multi.to_text().contains("NOT directly comparable"));

    // Graded, but only part of the list: still two standards in one list.
    multi.ranked = vec![];
    multi.triage.graded = 1;
    assert!(multi.to_text().contains("NOT directly comparable"));
}

#[test]
fn a_graded_report_says_how_much_moved() {
    let text = graded_report(Outcome {
        graded: 9,
        changed: 4,
        note: String::new(),
    })
    .to_text();
    assert!(text.contains("re-graded across the whole list"), "{text}");
    assert!(text.contains("4 of 9"), "{text}");
}

#[test]
fn a_moved_grade_is_shown_with_what_it_moved_from_and_why() {
    use bugsleuth_domain::{Finding, FindingId, ModelId, RawFinding, Severity, VerifiedAnchor};
    use bugsleuth_judge::{Cluster, Ranked};

    let mut cluster = Cluster {
        findings: vec![Finding::new(
            FindingId::new("f"),
            Lane::Ux,
            ModelId::new("claude:sonnet"),
            RawFinding {
                title: "t".into(),
                severity: Severity::High,
                file: "a.rs".into(),
                line: 1,
                snippet: "x".into(),
                explanation: "e".into(),
                failure_scenario: "f".into(),
                fix: Default::default(),
            },
            VerifiedAnchor {
                file: "a.rs".into(),
                line: 1,
                claimed_line: 1,
                snippet: "x".into(),
            },
        )],
        agreement: 1,
        triaged: Some(Severity::Low),
        triage_reason: Some("cosmetic only, nothing is lost".into()),
    };
    let unchanged = RunReport {
        ranked: vec![Ranked {
            position: 1,
            cluster: cluster.clone(),
        }],
        triage: crate::triage::Outcome {
            graded: 1,
            changed: 1,
            note: String::new(),
        },
        swept: vec![Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Ux,
            findings: 1,
            rejected: 0,
            salvaged: false,
        }],
        gaps: vec![],
    };
    let text = unchanged.to_text();
    assert!(text.contains("[LOW]"), "{text}");
    assert!(text.contains("said high"), "{text}");
    assert!(text.contains("cosmetic only"), "{text}");

    // An unmoved grade needs no annotation - noise is its own defect.
    cluster.triaged = Some(Severity::High);
    let same = RunReport {
        ranked: vec![Ranked {
            position: 1,
            cluster,
        }],
        ..unchanged
    };
    assert!(!same.to_text().contains("re-graded:"), "{}", same.to_text());
}

#[test]
fn rejected_findings_are_counted_in_the_sweep_line() {
    // The rejection rate is the headline measure of whether a model's
    // claims about this repository can be trusted; hiding it reads as a
    // clean record.
    let with_rejects = RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept: vec![Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Correctness,
            findings: 3,
            rejected: 2,
            salvaged: false,
        }],
        gaps: vec![],
    };
    let text = with_rejects.to_text();
    assert!(text.contains("2 rejected as unverifiable"), "{text}");
    // And a clean sweep says nothing about rejection at all.
    assert!(
        !report_with_no_findings().to_text().contains("rejected"),
        "clean sweep mentions rejection"
    );
}

/// A report with one clean sweep and nothing found.
fn report_with_no_findings() -> RunReport {
    report(Vec::new())
}

#[test]
fn a_report_says_what_the_method_could_not_see_even_when_every_lane_ran() {
    // A clean-looking list of defects with nothing said about the method's own
    // blind spots reads as "these are the problems". Two of these limits were
    // learned rather than reasoned: a real contract defect turned out to be a
    // disagreement with a remote server no reader of the code could see, and
    // "dependency risk" was never a check against any published advisory.
    let text = report(Vec::new()).to_text();
    assert!(text.contains("could not see"), "{text}");
    assert!(text.contains("Nothing was run"), "{text}");
    assert!(text.contains("vulnerability database"), "{text}");
}

#[test]
fn a_salvaged_sweep_does_not_read_like_a_clean_one() {
    // A review cut off partway and asked afterwards what it had can return a
    // short list. Rendered identically to a thorough sweep that found little,
    // it would be read as reassurance rather than as a truncated review.
    let salvaged = RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept: vec![Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Security,
            findings: 1,
            rejected: 0,
            salvaged: true,
        }],
        gaps: vec![],
    };
    let text = salvaged.to_text();
    assert!(text.contains("RECOVERED"), "{text}");
    assert!(text.contains("as far as it got"), "{text}");
    // An ordinary sweep says nothing of the sort.
    assert!(!report(Vec::new()).to_text().contains("RECOVERED"));
}

#[test]
fn a_run_that_used_an_unsandboxable_vendor_says_so_in_the_report() {
    // The start-up warning scrolled past twenty minutes ago. Whether an agent
    // with the user's own permissions was pointed at untrusted code belongs in
    // the written record, beside the findings it produced.
    let with_kilo = RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept: vec![Swept {
            model: "kilo:kimi".into(),
            lane: Lane::Security,
            findings: 1,
            rejected: 0,
            salvaged: false,
        }],
        gaps: vec![],
    };
    let text = with_kilo.to_text();
    assert!(text.contains("Caution:"), "{text}");
    assert!(text.contains("nothing else"), "{text}");

    // And a run of only sandboxable vendors says nothing, or the caution
    // becomes background noise nobody reads.
    assert!(!report(Vec::new()).to_text().contains("Caution:"));
}

#[test]
fn agreement_is_counted_against_models_not_sweeps() {
    // Two models covering two lanes is four sweeps, and a defect found by one
    // of them printed "found by 1 of 4 models" - understating agreement against
    // a total that never existed. Only models that swept the defect's own lane
    // could have found it.
    let two_models_two_lanes = vec![
        Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Correctness,
            findings: 1,
            rejected: 0,
            salvaged: false,
        },
        Swept {
            model: "codex:".into(),
            lane: Lane::Correctness,
            findings: 0,
            rejected: 0,
            salvaged: false,
        },
        Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Ux,
            findings: 0,
            rejected: 0,
            salvaged: false,
        },
        Swept {
            model: "codex:".into(),
            lane: Lane::Ux,
            findings: 0,
            rejected: 0,
            salvaged: false,
        },
    ];
    let report = RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept: two_models_two_lanes,
        gaps: vec![],
    };
    assert_eq!(report.models_on("correctness"), 2);
    assert_eq!(report.models_on("ux"), 2);
    // A lane nobody swept has no models, rather than borrowing another lane's.
    assert_eq!(report.models_on("security"), 0);
}

#[test]
fn repeating_a_model_does_not_inflate_the_model_count() {
    // Two passes of one model is two sweeps and still one model. Counting
    // sweeps would say a lone finding was "1 of 2 models" when no second model
    // ever looked.
    let repeated = RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept: vec![
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Correctness,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Correctness,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
        ],
        gaps: vec![],
    };
    assert_eq!(repeated.models_on("correctness"), 1);
}
