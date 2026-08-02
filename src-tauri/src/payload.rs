//! The structured half of the run-finished event.
//!
//! The window used to receive only rendered text, which made the frontend a
//! terminal emulator: it could show the report but not act on any single part
//! of it — no per-defect copy, no severity styling, no expanding one finding.
//! This shapes the same data the text is rendered from, so the window can do
//! all three without a second source of truth growing on the TypeScript side.

use bugsleuth_engine::orchestrate::RunReport;
use serde_json::{Value, json};

/// Every ranked defect, in report order, as the window will show it.
pub fn findings(report: &RunReport) -> Vec<Value> {
    report
        .ranked
        .iter()
        .map(|entry| {
            let cluster = &entry.cluster;
            let finding = cluster.representative();
            // The re-grade is sent as data, not prose: the card decides how to
            // show "the finder said high, the triage pass said low".
            let regraded = cluster
                .triaged
                .is_some_and(|graded| graded != cluster.claimed_severity());
            json!({
                "position": entry.position,
                "severity": cluster.severity().as_str(),
                "claimedSeverity": regraded.then(|| cluster.claimed_severity().as_str()),
                "triageReason": if regraded { cluster.triage_reason.clone() } else { None },
                "title": finding.title,
                "file": finding.anchor.file,
                "line": finding.anchor.line,
                "agreement": cluster.agreement,
                "models": cluster.models().iter().map(|m| m.to_string()).collect::<Vec<_>>(),
                "explanation": finding.explanation,
                "failure": finding.failure_scenario,
                "fixApproach": finding.fix.approach,
                "snippet": finding.anchor.snippet,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::{
        Finding, FindingId, Lane, ModelId, RawFinding, Severity, VerifiedAnchor,
    };
    use bugsleuth_engine::orchestrate::Swept;
    use bugsleuth_engine::{Cluster, Ranked};

    fn report(triaged: Option<Severity>, reason: Option<&str>) -> RunReport {
        RunReport {
            ranked: vec![Ranked {
                position: 1,
                cluster: Cluster {
                    findings: vec![Finding::new(
                        FindingId::new("f"),
                        Lane::Ux,
                        ModelId::new("claude:sonnet"),
                        RawFinding {
                            title: "the title".into(),
                            severity: Severity::High,
                            file: "src/a.ts".into(),
                            line: 10,
                            snippet: "code()".into(),
                            explanation: "why".into(),
                            failure_scenario: "boom".into(),
                            fix: Default::default(),
                        },
                        VerifiedAnchor {
                            file: "src/a.ts".into(),
                            line: 10,
                            claimed_line: 10,
                            snippet: "code()".into(),
                        },
                    )],
                    agreement: 1,
                    triaged,
                    triage_reason: reason.map(String::from),
                },
            }],
            triage: Default::default(),
            swept: vec![Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Ux,
                findings: 1,
                rejected: 0,
                salvaged: false,
            }],
            gaps: vec![],
        }
    }

    #[test]
    fn a_regrade_reaches_the_window_as_data_with_both_grades_and_the_reason() {
        let cards = findings(&report(Some(Severity::Low), Some("cosmetic")));
        assert_eq!(cards[0]["severity"], "low");
        assert_eq!(cards[0]["claimedSeverity"], "high");
        assert_eq!(cards[0]["triageReason"], "cosmetic");
    }

    #[test]
    fn an_unmoved_grade_sends_no_regrade_fields_so_the_card_stays_quiet() {
        // A reason without a moved grade would show a judgement that changed
        // nothing, and the card cannot know that.
        let cards = findings(&report(Some(Severity::High), Some("agrees")));
        assert_eq!(cards[0]["severity"], "high");
        assert!(cards[0]["claimedSeverity"].is_null());
        assert!(cards[0]["triageReason"].is_null());
    }
}
