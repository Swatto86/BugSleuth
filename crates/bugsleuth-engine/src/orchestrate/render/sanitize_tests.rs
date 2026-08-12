//! Report text must not carry raw control characters from model/repo prose.

use crate::orchestrate::{RunReport, Swept};
use bugsleuth_domain::{Finding, FindingId, Lane, ModelId, RawFinding, Severity, VerifiedAnchor};
use bugsleuth_judge::{Cluster, Ranked};

fn finding(title: &str) -> Finding {
    Finding::new(
        FindingId::new("f"),
        Lane::Ux,
        ModelId::new("claude:sonnet"),
        RawFinding {
            title: title.into(),
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
    )
}

fn swept() -> Swept {
    Swept {
        model: "claude:sonnet".into(),
        lane: Lane::Ux,
        commit: Some("aaaaaaaa".into()),
        cache_revision: Some("aaaaaaaa".into()),
        scope: None,
        usage: None,
        findings: 1,
        rejected: 0,
        salvaged: false,
    }
}

#[test]
fn a_moved_grade_reason_cannot_carry_raw_ansi_into_the_report() {
    let esc = char::from(27);
    let cluster = Cluster {
        findings: vec![finding("t")],
        agreement: 1,
        acknowledged: None,
        triaged: Some(Severity::Low),
        triage_reason: Some(format!("cosmetic {esc}[2J wipe")),
    };
    let text = RunReport {
        ranked: vec![Ranked {
            position: 1,
            cluster,
        }],
        triage: crate::triage::Outcome {
            graded: 1,
            changed: 1,
            note: String::new(),
        },
        swept: vec![swept()],
        gaps: vec![],
        cancelled: false,
    }
    .to_text();
    assert!(!text.contains(esc), "{text}");
    assert!(text.contains("<0x1b>"), "{text}");
}

#[test]
fn already_documented_quotes_cannot_carry_raw_ansi_into_the_report() {
    let esc = char::from(27);
    let cluster = Cluster {
        findings: vec![finding("known")],
        agreement: 1,
        acknowledged: Some(format!("doc {esc}[2J hide")),
        triaged: Some(Severity::Low),
        triage_reason: None,
    };
    let text = RunReport {
        ranked: vec![Ranked {
            position: 1,
            cluster,
        }],
        triage: Default::default(),
        swept: vec![swept()],
        gaps: vec![],
        cancelled: false,
    }
    .to_text();
    assert!(text.contains("ALREADY DOCUMENTED"), "{text}");
    assert!(!text.contains(esc), "{text}");
    assert!(text.contains("<0x1b>"), "{text}");
}
