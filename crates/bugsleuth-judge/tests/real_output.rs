//! Clustering measured against real cross-vendor output.
//!
//! `tests/data/` holds the actual JSON from a Claude sweep and a Codex sweep of
//! the same lane on the same repository. It is real model output, not invented
//! examples, because the thing this crate has to get right — telling "the same
//! defect in different words" from "two different defects on adjacent lines" —
//! only looks easy until you see what models actually write.
//!
//! Ground truth below was labelled by hand from the source under review. The
//! assertions name specific pairs rather than a total count: whether two
//! overlapping descriptions of the same broken function are one defect or two
//! is genuinely arguable, but whether *these two specific findings* describe the
//! same bug is not.

use bugsleuth_domain::Finding;
use bugsleuth_judge::{Cluster, cluster};
use serde::Deserialize;

#[derive(Deserialize)]
struct SweepFile {
    findings: Vec<Finding>,
}

fn load() -> Vec<Finding> {
    let mut all = Vec::new();
    for name in ["correctness-claude.json", "correctness-codex.json"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let file: SweepFile = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{} is not a sweep report: {e}", path.display()));
        all.extend(file.findings);
    }
    all
}

/// The cluster containing the finding whose title contains `needle`.
fn cluster_with<'a>(clusters: &'a [Cluster], needle: &str) -> &'a Cluster {
    clusters
        .iter()
        .find(|c| c.findings.iter().any(|f| f.title.contains(needle)))
        .unwrap_or_else(|| panic!("no finding with a title containing {needle:?}"))
}

/// Whether two findings, identified by substrings of their titles, ended up
/// grouped as one defect.
fn grouped(clusters: &[Cluster], left: &str, right: &str) -> bool {
    let cluster = cluster_with(clusters, left);
    cluster.findings.iter().any(|f| f.title.contains(right))
}

#[test]
fn the_two_vendors_findings_load_and_are_the_expected_size() {
    let findings = load();
    assert_eq!(
        findings.len(),
        16,
        "test data changed; the ground truth below was labelled for 16 findings"
    );
}

#[test]
fn the_same_defect_described_by_each_vendor_is_merged() {
    let clusters = cluster(load());

    // Same line, same bug, different grammar: "remove_stock underflows quantity"
    // against "Removing more stock than available underflows the quantity".
    assert!(
        grouped(&clusters, "remove_stock underflows", "Removing more stock"),
        "the unchecked-subtraction defect was reported as two separate defects"
    );

    // "divides by zero on an empty inventory" against "Calculating the average
    // price of an empty inventory panics".
    assert!(
        grouped(&clusters, "divides by zero", "empty inventory panics"),
        "the divide-by-zero defect was reported as two separate defects"
    );

    // "top_by_value panics when n exceeds" against "Requesting more top items
    // than exist panics".
    assert!(
        grouped(
            &clusters,
            "top_by_value panics",
            "Requesting more top items"
        ),
        "the slice-bounds defect was reported as two separate defects"
    );

    // "discount tiers are off by one" against "Bulk discounts are not applied at
    // their documented thresholds".
    assert!(
        grouped(&clusters, "off by one", "Bulk discounts"),
        "the threshold off-by-one defect was reported as two separate defects"
    );

    // "apply_discount underflows for a percent_off greater than 100" against
    // "Discount percentages above 100 underflow the result".
    assert!(
        grouped(
            &clusters,
            "apply_discount underflows",
            "Discount percentages above 100"
        ),
        "the discount-underflow defect was reported as two separate defects"
    );
}

#[test]
fn genuinely_different_defects_are_not_merged() {
    let clusters = cluster(load());

    // Reported one line apart in the same function, and not the same bug: one
    // is a divide-by-zero on an empty inventory, the other an integer overflow
    // while summing. Anchor proximity alone would merge these.
    assert!(
        !grouped(&clusters, "divides by zero", "accumulation can overflow"),
        "a divide-by-zero and an overflow in the same function were merged"
    );

    // Likewise: a panic when n exceeds the length, versus an overflow computing
    // the sort key, one line apart.
    assert!(
        !grouped(&clusters, "top_by_value panics", "ranking overflows"),
        "a bounds panic and a multiplication overflow were merged"
    );
}

#[test]
fn defects_only_one_vendor_found_survive_the_merge() {
    let clusters = cluster(load());

    // Codex alone reported these three overflow paths. A merge that loses a
    // finding because only one model saw it would defeat the point of running
    // more than one model.
    for title in [
        "accumulation can overflow",
        "ranking overflows",
        "Basket total overflows",
    ] {
        let found = clusters
            .iter()
            .any(|c| c.findings.iter().any(|f| f.title.contains(title)));
        assert!(found, "{title:?} was lost during merging");
    }
}

#[test]
fn agreement_counts_distinct_vendors() {
    let clusters = cluster(load());

    let shared = cluster_with(&clusters, "divides by zero");
    assert_eq!(shared.agreement, 2, "both vendors found this one");

    let codex_only = cluster_with(&clusters, "accumulation can overflow");
    assert_eq!(codex_only.agreement, 1, "only Codex found this one");
}

#[test]
fn merging_reduces_the_list_without_collapsing_it() {
    let clusters = cluster(load());
    // 16 raw findings. Five defects were found by both vendors, so a correct
    // merge lands near eleven. The bounds are deliberately loose: they catch a
    // measure that has stopped merging anything (16) or one that has started
    // merging everything nearby, without pinning an arguable exact number.
    assert!(
        (9..=12).contains(&clusters.len()),
        "expected roughly 11 distinct defects from 16 findings, got {}",
        clusters.len()
    );
}

// ── A second corpus: real output from a real repository ──────────────────────
//
// The fixture above is a small purpose-built crate. This one is two vendors
// sweeping a real 27-file crate, and it exposed a clustering miss the fixture
// could not: the fixture's findings all landed within a line or two of each
// other, so the anchor window was never stressed.

fn load_real_repo() -> Vec<Finding> {
    let mut all = Vec::new();
    for name in ["real-repo-claude.json", "real-repo-codex.json"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let file: SweepFile = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{} is not a sweep report: {e}", path.display()));
        all.extend(file.findings);
    }
    all
}

#[test]
fn one_defect_anchored_five_lines_apart_by_two_vendors_still_merges() {
    // Both vendors independently found the same bug in `snap_all_day_to_midnight`
    // — a timezone heuristic that assumes offsets are under 12 hours — but
    // anchored it at the signature and at the offending comparison, 5 lines
    // apart. With the original 3-line window this was reported as two separate
    // defects, each with no agreement, instead of one found by both.
    let clusters = cluster(load_real_repo());
    assert!(
        grouped(
            &clusters,
            "midnight-snap misclassifies",
            "shift one day late"
        ),
        "the same timezone defect was reported as two defects with no agreement"
    );
    assert_eq!(cluster_with(&clusters, "shift one day late").agreement, 2);
}

#[test]
fn unrelated_defects_in_the_same_real_crate_stay_separate() {
    let clusters = cluster(load_real_repo());
    // Different files entirely, and nothing to do with each other.
    assert!(!grouped(
        &clusters,
        "Partial refresh-token writes",
        "attachments()"
    ));
    assert!(!grouped(&clusters, "attachments()", "shift one day late"));
}

#[test]
fn the_real_repo_corpus_merges_four_findings_into_three_defects() {
    let clusters = cluster(load_real_repo());
    assert_eq!(load_real_repo().len(), 4);
    assert_eq!(
        clusters.len(),
        3,
        "expected the two timezone findings to merge and the other two to stand alone"
    );
}
