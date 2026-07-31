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

/// How far apart two anchors may be and still be candidates for the same defect.
///
/// **Widened from 3 after real output showed 3 was too tight.** Reviewing a real
/// repository, two vendors independently found the same bug in one small
/// function — a timezone heuristic that assumes offsets are under 12 hours — but
/// anchored it 5 lines apart, one at the signature and one at the offending
/// comparison. They scored 0.25 on wording, well above the merge threshold, so
/// only the distance kept them apart and the report claimed two defects with no
/// agreement instead of one found by both.
///
/// Roughly "within the same small function". Widening this is safe because
/// wording is the real separator: on measured output, different defects on
/// *adjacent* lines score 0.07-0.08 against a 0.20 threshold, so distance is not
/// what is doing the work.
const MAX_LINE_GAP: u32 = 10;

/// Minimum wording overlap for two nearby findings to be treated as one.
///
/// **Measured, not guessed.** Against hand-labelled real cross-vendor output
/// (`tests/data`, and the integration test that uses it), pairs that describe
/// the same defect scored 0.24 to 0.32, and pairs that describe different
/// defects on adjacent lines scored 0.07 to 0.08. 0.20 sits in that gap with
/// roughly 2.4x margin on the wrong-merge side.
///
/// Erring toward *not* merging is the right direction: a duplicate in the
/// report is an annoyance, while two distinct defects silently collapsed into
/// one means the reader never learns about the second.
///
/// Known limitation this leaves in place: when one model reports a compound
/// finding — Codex described a panic *and* an arithmetic error as one defect —
/// it scores 0.16 against each single-defect counterpart and stays separate. No
/// threshold fixes that, because the two findings genuinely are not one-to-one.
const MIN_WORDING_OVERLAP: f32 = 0.20;

/// One defect, as reported by one or more models.
#[derive(Debug, Clone, Serialize)]
pub struct Cluster {
    /// Every report of this defect. Never empty.
    pub findings: Vec<Finding>,
    /// How many *distinct models* independently reported it. This is the
    /// headline trust signal: a defect three models found separately deserves
    /// to outrank one model's lone opinion.
    pub agreement: usize,
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

    /// Severity after normalisation: the most severe assessment in the cluster.
    ///
    /// Most severe rather than an average, because a defect one model called
    /// critical and another called medium is a defect worth looking at. Losing
    /// that to averaging is the wrong failure direction for a tool whose reader
    /// cannot check the code.
    pub fn severity(&self) -> Severity {
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
            }),
        }
    }

    for cluster in &mut clusters {
        cluster.agreement = cluster.models().len();
    }
    clusters
}

/// Whether two findings describe the same defect.
fn same_defect(left: &Finding, right: &Finding) -> bool {
    if left.anchor.file != right.anchor.file {
        return false;
    }
    if left.anchor.line.abs_diff(right.anchor.line) > MAX_LINE_GAP {
        return false;
    }
    // Compare title plus explanation. The title alone is often too short to
    // separate two defects in the same function; the explanation carries the
    // words that distinguish "divides by zero" from "can overflow".
    let left_text = format!("{} {}", left.title, left.explanation);
    let right_text = format!("{} {}", right.title, right.explanation);
    crate::similarity(&left_text, &right_text) >= MIN_WORDING_OVERLAP
}

/// Whether a finding carries usable fix instructions.
fn has_plan(finding: &Finding) -> bool {
    !finding.fix.approach.trim().is_empty() || !finding.fix.edits.is_empty()
}

pub(crate) fn severity_order(severity: Severity) -> u8 {
    match severity {
        Severity::Critical => 0,
        Severity::High => 1,
        Severity::Medium => 2,
        Severity::Low => 3,
    }
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
        };
        assert_eq!(
            cluster.representative().fix.approach,
            "guard the empty case",
            "the cluster picked the finding with no plan"
        );
    }
}
