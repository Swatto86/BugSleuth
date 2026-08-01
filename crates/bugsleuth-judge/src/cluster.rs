//! Grouping findings that describe the same defect.
//!
//! Two gates, and both must pass. Findings must be anchored to the same file
//! within a few lines of each other, *and* describe the defect in similar
//! enough words. Either gate alone gives the wrong answer: anchors alone merge
//! distinct defects that happen to sit close together, and wording alone merges
//! the same *kind* of defect wherever it occurs — every unchecked subtraction in
//! a codebase reads much the same.

use bugsleuth_domain::{Finding, ModelId, Severity};
use serde::Serialize;

pub(crate) mod pairing;

pub(crate) use pairing::severity_order;
use pairing::{has_plan, same_defect};

/// One defect, as reported by one or more models.
#[derive(Debug, Clone, Serialize)]
pub struct Cluster {
    /// Every report of this defect. Never empty.
    pub findings: Vec<Finding>,
    /// How many *distinct models* independently reported it. This is the
    /// headline trust signal: a defect three models found separately deserves
    /// to outrank one model's lone opinion.
    pub agreement: usize,
    /// Severity decided by a triage pass that saw every defect at once, if one
    /// ran. Kept beside the models' own grades rather than overwriting them:
    /// severity decides the order of the whole report, so a reader who cannot
    /// check the code is owed the fact that it was changed, and to what from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triaged: Option<Severity>,
    /// The one-sentence consequence the triage pass gave for its grade. Shown
    /// with a re-grade so a changed severity is a stated judgement, not a
    /// silent contradiction of what the finding's own model said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage_reason: Option<String>,
}

impl Cluster {
    /// The report to show. The most severe one, so a cluster is never presented
    /// more mildly than its worst assessment.
    /// The finding that speaks for the cluster.
    ///
    /// Most severe first, and among equals the one that actually came with a
    /// fix plan. Two models finding the same defect and only one explaining how
    /// to fix it is common; picking the silent one would throw away the only
    /// instructions in the cluster while changing nothing else about the report.
    pub fn representative(&self) -> &Finding {
        self.findings
            .iter()
            .min_by_key(|f| (severity_order(f.severity), usize::from(!has_plan(f))))
            .unwrap_or(&self.findings[0])
    }

    /// Severity after normalisation: a triage grade if one was made, otherwise
    /// the most severe assessment in the cluster.
    ///
    /// Most severe rather than an average, because a defect one model called
    /// critical and another called medium is a defect worth looking at. Losing
    /// that to averaging is the wrong failure direction for a tool whose reader
    /// cannot check the code.
    ///
    /// A triage grade wins over both because it is the only one made with the
    /// rest of the report in view. Each model grades its own findings in
    /// isolation, and a lane whose worst defect is an unhandled edge case will
    /// still call one of them high — there is nothing else there to be worse.
    pub fn severity(&self) -> Severity {
        self.triaged
            .unwrap_or_else(|| self.representative().severity)
    }

    /// What the models themselves called it, before any triage pass.
    pub fn claimed_severity(&self) -> Severity {
        self.representative().severity
    }

    /// Distinct models that reported this defect, in the order first seen.
    pub fn models(&self) -> Vec<&ModelId> {
        let mut seen: Vec<&ModelId> = Vec::new();
        for finding in &self.findings {
            if !seen.contains(&&finding.model) {
                seen.push(&finding.model);
            }
        }
        seen
    }
}

/// Group findings that describe the same defect.
///
/// Order-independent by construction: a finding joins the first cluster it
/// matches, and matching is symmetric, so the same input in any order produces
/// the same grouping.
pub fn cluster(findings: Vec<Finding>) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();

    for finding in findings {
        match clusters.iter_mut().find(|cluster| {
            cluster
                .findings
                .iter()
                .any(|other| same_defect(other, &finding))
        }) {
            Some(cluster) => cluster.findings.push(finding),
            None => clusters.push(Cluster {
                findings: vec![finding],
                agreement: 0,
                triaged: None,
                triage_reason: None,
            }),
        }
    }

    for cluster in &mut clusters {
        cluster.agreement = cluster.models().len();
    }
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::{FindingId, LaneId, VerifiedAnchor};

    fn finding(model: &str, file: &str, line: u32, title: &str, explanation: &str) -> Finding {
        Finding {
            id: FindingId::new(format!("{model}-{line}")),
            lane: LaneId::new("correctness"),
            model: ModelId::new(model),
            title: title.into(),
            severity: Severity::High,
            anchor: VerifiedAnchor {
                file: file.into(),
                line,
                claimed_line: line,
                snippet: "code".into(),
            },
            explanation: explanation.into(),
            failure_scenario: "f".into(),
            fix: Default::default(),
        }
    }

    #[test]
    fn two_vendors_reporting_the_same_defect_become_one_cluster_with_agreement_two() {
        let findings = vec![
            finding(
                "claude:sonnet",
                "src/inventory.rs",
                42,
                "average_price divides by zero on an empty inventory",
                "There is no check for an empty inventory before dividing by the item count.",
            ),
            finding(
                "codex:",
                "src/inventory.rs",
                43,
                "Calculating the average price of an empty inventory panics",
                "An empty inventory has length zero, so this integer division panics.",
            ),
        ];
        let clusters = cluster(findings);
        assert_eq!(clusters.len(), 1, "the same defect was not merged");
        assert_eq!(clusters[0].agreement, 2);
    }

    #[test]
    fn two_different_defects_one_line_apart_stay_separate() {
        // The case that anchor-proximity clustering alone gets wrong.
        let findings = vec![
            finding(
                "claude:sonnet",
                "src/inventory.rs",
                42,
                "average_price divides by zero on an empty inventory",
                "There is no check for an empty inventory before dividing by the item count.",
            ),
            finding(
                "codex:",
                "src/inventory.rs",
                42,
                "Average price accumulation can overflow",
                "Summing prices into u64 is unchecked and can exceed the maximum value.",
            ),
        ];
        assert_eq!(cluster(findings).len(), 2, "distinct defects were merged");
    }

    #[test]
    fn the_same_kind_of_defect_in_different_files_stays_separate() {
        let findings = vec![
            finding(
                "a",
                "src/one.rs",
                10,
                "unchecked subtraction underflows",
                "no guard",
            ),
            finding(
                "b",
                "src/two.rs",
                10,
                "unchecked subtraction underflows",
                "no guard",
            ),
        ];
        assert_eq!(cluster(findings).len(), 2);
    }

    #[test]
    fn one_model_reporting_a_defect_twice_counts_as_agreement_of_one() {
        let findings = vec![
            finding(
                "a",
                "src/one.rs",
                10,
                "slice index panics",
                "no bound check",
            ),
            finding(
                "a",
                "src/one.rs",
                11,
                "slice index panics",
                "no bound check",
            ),
        ];
        let clusters = cluster(findings);
        assert_eq!(clusters.len(), 1);
        assert_eq!(
            clusters[0].agreement, 1,
            "agreement must count distinct models, not findings"
        );
    }

    #[test]
    fn clustering_does_not_depend_on_the_order_findings_arrive_in() {
        let make = || {
            vec![
                finding(
                    "a",
                    "src/one.rs",
                    10,
                    "slice index panics",
                    "no bound check",
                ),
                finding(
                    "b",
                    "src/one.rs",
                    11,
                    "slice indexing panics",
                    "missing bound check",
                ),
                finding("c", "src/two.rs", 99, "divides by zero", "empty collection"),
            ]
        };
        let forward = cluster(make()).len();
        let mut reversed = make();
        reversed.reverse();
        assert_eq!(forward, cluster(reversed).len());
    }

    #[test]
    fn a_cluster_reports_the_most_severe_assessment_not_the_mildest() {
        let mut low = finding(
            "a",
            "src/one.rs",
            10,
            "slice index panics",
            "no bound check",
        );
        low.severity = Severity::Low;
        let mut critical = finding(
            "b",
            "src/one.rs",
            10,
            "slice index panics",
            "no bound check",
        );
        critical.severity = Severity::Critical;

        let clusters = cluster(vec![low, critical]);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].severity(), Severity::Critical);
    }

    #[test]
    fn the_cluster_speaks_through_whichever_finding_brought_a_fix_plan() {
        // Two models finding the same defect and only one explaining how to fix
        // it is the common case. Picking the silent one loses the only
        // instructions in the cluster and changes nothing else, so it would go
        // unnoticed.
        let mut silent = finding("claude:sonnet", "a", 10, "the same defect", "why");
        silent.fix = Default::default();
        let mut explained = finding("codex:", "a", 10, "the same defect", "why");
        explained.fix = bugsleuth_domain::FixPlan {
            approach: "guard the empty case".into(),
            ..Default::default()
        };

        let cluster = Cluster {
            findings: vec![silent, explained],
            agreement: 2,
            triaged: None,
            triage_reason: None,
        };
        assert_eq!(
            cluster.representative().fix.approach,
            "guard the empty case",
            "the cluster picked the finding with no plan"
        );
    }
}

#[cfg(test)]
mod triage_tests {
    use super::*;
    use bugsleuth_domain::{Finding, FindingId, Lane, ModelId, RawFinding, VerifiedAnchor};

    fn cluster_at(severity: Severity) -> Cluster {
        Cluster {
            findings: vec![Finding::new(
                FindingId::new("f"),
                Lane::Correctness,
                ModelId::new("claude:sonnet"),
                RawFinding {
                    title: "t".into(),
                    severity,
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
            triaged: None,
            triage_reason: None,
        }
    }

    #[test]
    fn a_triage_grade_decides_the_severity_and_the_original_is_still_readable() {
        // The whole report is ordered by severity, so a reader who cannot check
        // the code has to be able to see that a grade was changed, and from what.
        let mut cluster = cluster_at(Severity::Critical);
        cluster.triaged = Some(Severity::Low);
        assert_eq!(cluster.severity(), Severity::Low);
        assert_eq!(cluster.claimed_severity(), Severity::Critical);
    }

    #[test]
    fn without_a_triage_pass_the_models_own_grade_stands() {
        // Triage costs quota and can fail. When it does not run, nothing about
        // the report may change.
        let cluster = cluster_at(Severity::High);
        assert_eq!(cluster.severity(), Severity::High);
        assert_eq!(cluster.severity(), cluster.claimed_severity());
    }
}
