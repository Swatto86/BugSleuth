//! Tests for clustering, in their own file only because the module plus
//! its tests crossed the hard line cap.

use super::*;
use bugsleuth_domain::{Finding, FindingId, Lane, LaneId, ModelId, RawFinding, VerifiedAnchor};

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
        acknowledged: None,
        triaged: None,
        triage_reason: None,
    };
    assert_eq!(
        cluster.representative().fix.approach,
        "guard the empty case",
        "the cluster picked the finding with no plan"
    );
}

fn in_lane(lane: &str, model: &str) -> Finding {
    Finding {
        id: FindingId::new(format!("{lane}-{model}")),
        lane: LaneId::new(lane),
        model: ModelId::new(model),
        title: "unchecked user input reaches the shell".into(),
        severity: Severity::High,
        anchor: VerifiedAnchor {
            file: "src/run.rs".into(),
            line: 40,
            claimed_line: 40,
            snippet: "code".into(),
        },
        explanation: "unchecked user input reaches the shell without validation".into(),
        failure_scenario: "f".into(),
        fix: Default::default(),
    }
}

/// Two lanes finding the same lines is two defects, not one found twice.
///
/// The defect: clustering compared file, line distance, wording and shared
/// identifiers, and never the lane. A correctness finding and a security
/// finding on the same function — worded alike, because they are describing
/// the same code — merged, and merging keeps one account. The other defect
/// vanished from the report entirely, which is the worst failure this judge
/// has: the reader never learns it exists.
#[test]
fn findings_from_different_lanes_are_never_merged() {
    let correctness = in_lane("correctness", "claude:sonnet");
    let security = in_lane("security", "codex:");
    let clusters = cluster(vec![correctness, security]);
    assert_eq!(
        clusters.len(),
        2,
        "a security defect was swallowed by a correctness one"
    );
}

/// The case merging exists for must keep working.
#[test]
fn two_models_in_one_lane_still_merge() {
    let a = in_lane("security", "claude:sonnet");
    let b = in_lane("security", "codex:");
    assert_eq!(cluster(vec![a, b]).len(), 1, "cross-vendor merging broke");
}

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
        acknowledged: None,
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

/// Every ordering of the same findings must produce the same report.
///
/// The defect: a finding joined the *first* cluster it matched, and
/// `same_defect` is symmetric but not transitive — wording similarity is not.
/// A middle finding matching two separate clusters merged them in one order and
/// left them apart in another, so which report a user saw depended on the order
/// the sweeps happened to finish in. The doc comment claimed the opposite,
/// which is how it survived being read.
///
/// **The premise is asserted, not assumed.** The first version of this test
/// used three wordings that all matched each other, so it passed on the broken
/// algorithm and proved nothing. If the three findings below ever stop forming
/// a chain, this fails on that rather than quietly going vacuous.
#[test]
fn the_grouping_does_not_depend_on_the_order_findings_arrive_in() {
    let a = worded(
        "a",
        10,
        "average_price divides by zero when the inventory is empty",
    );
    let b = worded(
        "b",
        12,
        "average_price divides by zero when the inventory is empty, and remove_stock          underflows when the quantity is larger than the stock",
    );
    let c = worded(
        "c",
        14,
        "remove_stock underflows when the quantity is larger than the stock held",
    );

    // A chain, not a triangle: B is the compound finding that bridges two
    // defects sharing no identifier of their own.
    assert!(
        pairing::same_defect(&a, &b),
        "A and B must match for this to test anything"
    );
    assert!(
        pairing::same_defect(&b, &c),
        "B and C must match for this to test anything"
    );
    assert!(
        !pairing::same_defect(&a, &c),
        "A and C must NOT match, or there is no chain"
    );

    let orders = [
        vec![a.clone(), b.clone(), c.clone()],
        vec![a.clone(), c.clone(), b.clone()],
        vec![c.clone(), b.clone(), a.clone()],
        vec![c.clone(), a.clone(), b.clone()],
    ];
    let groupings: Vec<Vec<usize>> = orders
        .into_iter()
        .map(|order| {
            let mut sizes: Vec<usize> = cluster(order).iter().map(|c| c.findings.len()).collect();
            sizes.sort_unstable();
            sizes
        })
        .collect();

    assert!(
        groupings.windows(2).all(|pair| pair[0] == pair[1]),
        "the same findings grouped differently depending on their order: {groupings:?}"
    );
}
fn worded(id: &str, line: u32, explanation: &str) -> Finding {
    Finding {
        id: FindingId::new(id),
        lane: LaneId::new("correctness"),
        model: ModelId::new("claude:sonnet"),
        title: "inventory arithmetic".into(),
        severity: Severity::High,
        anchor: VerifiedAnchor {
            file: "src/price.rs".into(),
            line,
            claimed_line: line,
            snippet: "code".into(),
        },
        explanation: explanation.into(),
        failure_scenario: "f".into(),
        fix: Default::default(),
    }
}
