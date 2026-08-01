//! Ordering merged defects.
//!
//! Severity first, then how many models independently found it. That order is
//! deliberate and worth stating: agreement is a strong signal about whether a
//! finding is *real*, but it says nothing about whether it *matters*. A
//! critical defect one model spotted still outranks a low-severity one that
//! three models all noticed.
//!
//! Agreement therefore breaks ties within a severity band rather than driving
//! the ranking. A vendor that reports everything would otherwise be able to
//! push its own noise up the list simply by agreeing with itself.

use serde::Serialize;

use crate::cluster::{Cluster, severity_order};

/// A cluster with its final position.
#[derive(Debug, Clone, Serialize)]
pub struct Ranked {
    /// 1-based position in the report.
    pub position: usize,
    pub cluster: Cluster,
}

/// Order clusters worst-first and number them.
pub fn rank(clusters: Vec<Cluster>) -> Vec<Ranked> {
    let mut clusters = clusters;
    clusters.sort_by(|a, b| {
        severity_order(a.severity())
            .cmp(&severity_order(b.severity()))
            .then_with(|| b.agreement.cmp(&a.agreement))
            // Then by location, so the order is stable across runs rather than
            // depending on which model happened to answer first.
            .then_with(|| {
                let (left, right) = (a.representative(), b.representative());
                left.anchor
                    .file
                    .cmp(&right.anchor.file)
                    .then_with(|| left.anchor.line.cmp(&right.anchor.line))
            })
    });

    clusters
        .into_iter()
        .enumerate()
        .map(|(index, cluster)| Ranked {
            position: index + 1,
            cluster,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::{Finding, FindingId, LaneId, ModelId, Severity, VerifiedAnchor};

    fn cluster_of(severity: Severity, agreement: usize, file: &str, line: u32) -> Cluster {
        let findings: Vec<Finding> = (0..agreement.max(1))
            .map(|index| Finding {
                id: FindingId::new(format!("{file}-{line}-{index}")),
                lane: LaneId::new("correctness"),
                model: ModelId::new(format!("model-{index}")),
                title: "t".into(),
                severity,
                anchor: VerifiedAnchor {
                    file: file.into(),
                    line,
                    claimed_line: line,
                    snippet: "code".into(),
                },
                explanation: "e".into(),
                failure_scenario: "f".into(),
                fix: Default::default(),
            })
            .collect();
        Cluster {
            findings,
            agreement,
        }
    }

    #[test]
    fn severity_outranks_agreement() {
        let ranked = rank(vec![
            cluster_of(Severity::Low, 3, "a.rs", 1),
            cluster_of(Severity::Critical, 1, "b.rs", 1),
        ]);
        assert_eq!(ranked[0].cluster.severity(), Severity::Critical);
        assert_eq!(ranked[0].position, 1);
    }

    #[test]
    fn agreement_breaks_ties_within_a_severity_band() {
        let ranked = rank(vec![
            cluster_of(Severity::High, 1, "a.rs", 1),
            cluster_of(Severity::High, 3, "b.rs", 1),
        ]);
        assert_eq!(ranked[0].cluster.agreement, 3);
        assert_eq!(ranked[1].cluster.agreement, 1);
    }

    #[test]
    fn identical_severity_and_agreement_order_by_location_so_runs_are_stable() {
        let forward = rank(vec![
            cluster_of(Severity::High, 1, "z.rs", 5),
            cluster_of(Severity::High, 1, "a.rs", 9),
        ]);
        let backward = rank(vec![
            cluster_of(Severity::High, 1, "a.rs", 9),
            cluster_of(Severity::High, 1, "z.rs", 5),
        ]);
        let files = |r: &[Ranked]| {
            r.iter()
                .map(|x| x.cluster.representative().anchor.file.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(files(&forward), files(&backward));
        assert_eq!(files(&forward), vec!["a.rs", "z.rs"]);
    }

    #[test]
    fn positions_are_one_based_and_contiguous() {
        let ranked = rank(vec![
            cluster_of(Severity::Low, 1, "a.rs", 1),
            cluster_of(Severity::High, 1, "b.rs", 1),
            cluster_of(Severity::Medium, 1, "c.rs", 1),
        ]);
        let positions: Vec<usize> = ranked.iter().map(|r| r.position).collect();
        assert_eq!(positions, vec![1, 2, 3]);
    }
}
