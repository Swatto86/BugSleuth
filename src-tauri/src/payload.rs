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
///
/// Each card carries its own standalone fix prompt. The bundle is already
/// offered whole and each defect is already written to its own file, but the
/// window could only hand over all of them at once — and handing one defect at
/// a time to a small local model is the workflow the handoff was built around.
pub fn findings(repo: &str, report: &RunReport) -> Vec<Value> {
    // Acknowledged findings are documented as deliberate decisions, not defects
    // to fix. They must not become a card carrying a standalone fix prompt — the
    // window would offer to have a model undo behaviour the code chose on
    // purpose, the same way the engine handoff excludes them.
    let actionable: Vec<&bugsleuth_engine::Ranked> = report
        .ranked
        .iter()
        .filter(|entry| entry.cluster.acknowledged.is_none())
        .collect();
    let total = actionable.last().map_or(0, |entry| entry.position);
    let sweeps = report.swept.len();
    actionable
        .iter()
        .map(|&entry| {
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
                "prompt": bugsleuth_engine::handoff::single(repo, entry, total, sweeps),
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
            cancelled: false,
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
                    acknowledged: None,
                    triaged,
                    triage_reason: reason.map(String::from),
                },
            }],
            triage: Default::default(),
            swept: vec![Swept {
                model: "claude:sonnet".into(),
                lane: Lane::Ux,
                commit: Some("aaaaaaaa".into()),
                cache_revision: Some("aaaaaaaa".into()),
                scope: None,
                excluded_paths: vec![],
                usage: None,
                findings: 1,
                rejected: 0,
                salvaged: false,
            }],
            gaps: vec![],
        }
    }

    #[test]
    fn a_regrade_reaches_the_window_as_data_with_both_grades_and_the_reason() {
        let cards = findings("C:/repo", &report(Some(Severity::Low), Some("cosmetic")));
        assert_eq!(cards[0]["severity"], "low");
        assert_eq!(cards[0]["claimedSeverity"], "high");
        assert_eq!(cards[0]["triageReason"], "cosmetic");
    }

    #[test]
    fn an_unmoved_grade_sends_no_regrade_fields_so_the_card_stays_quiet() {
        // A reason without a moved grade would show a judgement that changed
        // nothing, and the card cannot know that.
        let cards = findings("C:/repo", &report(Some(Severity::High), Some("agrees")));
        assert_eq!(cards[0]["severity"], "high");
        assert!(cards[0]["claimedSeverity"].is_null());
        assert!(cards[0]["triageReason"].is_null());
    }

    #[test]
    fn acknowledged_findings_are_not_cards() {
        // An acknowledged finding is a documented decision, not a defect to fix.
        // It must not reach the window as a card carrying a standalone fix
        // prompt, or the window would offer to have a model undo it.
        let mut report = report(None, None);
        let mut actionable = report.ranked[0].clone();
        actionable.position = 2;
        report.ranked[0].cluster.acknowledged = Some("documented on purpose".into());
        report.ranked.push(actionable);

        let cards = findings("C:/repo", &report);
        assert_eq!(cards.len(), 1, "an acknowledged finding produced a card");
        assert_eq!(cards[0]["position"], 2, "the wrong finding survived");
        let prompt = cards[0]["prompt"].as_str().unwrap_or_default();
        assert!(prompt.contains("defect 2 of 2"), "{prompt}");
    }

    #[test]
    fn each_card_carries_a_standalone_prompt_for_that_one_defect() {
        // Handing one defect at a time to a small local model is the workflow
        // the handoff was built around; the window could only hand over the
        // whole bundle.
        let cards = findings("C:/repo", &report(None, None));
        let prompt = cards[0]["prompt"].as_str().unwrap_or_default();
        assert!(
            prompt.contains("the title"),
            "the defect is not in its prompt"
        );
        assert!(prompt.contains("defect 1 of 1"), "{prompt}");
        assert!(prompt.contains("C:/repo"), "the repository is not named");
    }
}
