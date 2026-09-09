//! Tests for how much of the matrix agreed about a defect.
//!
//! Split from `annotations.rs` at the hard line cap, on its own subject: the
//! others are about what a report says regarding its own method, and these are
//! about one number in it — "found by N of M models" — which has been wrong in
//! two different ways and is what a reader weighs a finding by.

use crate::orchestrate::{RunReport, Swept};
use bugsleuth_domain::Lane;

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
            commit: Some("aaaaaaaa".into()),
            cache_revision: Some("aaaaaaaa".into()),
            scope: None,
            excluded_paths: vec![],
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
            excluded_paths: vec![],
            usage: None,
            findings: 0,
            rejected: 0,
            salvaged: false,
        },
        Swept {
            model: "claude:sonnet".into(),
            lane: Lane::Ux,
            commit: Some("aaaaaaaa".into()),
            cache_revision: Some("aaaaaaaa".into()),
            scope: None,
            excluded_paths: vec![],
            usage: None,
            findings: 0,
            rejected: 0,
            salvaged: false,
        },
        Swept {
            model: "codex:".into(),
            lane: Lane::Ux,
            commit: Some("aaaaaaaa".into()),
            cache_revision: Some("aaaaaaaa".into()),
            scope: None,
            excluded_paths: vec![],
            usage: None,
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
        interrupted: None,
        cancelled: false,
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
                commit: Some("aaaaaaaa".into()),
                cache_revision: Some("aaaaaaaa".into()),
                scope: None,
                excluded_paths: vec![],
                usage: None,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
            Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Correctness,
                commit: Some("aaaaaaaa".into()),
                cache_revision: Some("aaaaaaaa".into()),
                scope: None,
                excluded_paths: vec![],
                usage: None,
                findings: 1,
                rejected: 0,
                salvaged: false,
            },
        ],
        gaps: vec![],
        interrupted: None,
        cancelled: false,
    };
    assert_eq!(repeated.models_on("correctness"), 1);
}
