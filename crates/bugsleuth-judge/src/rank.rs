//! Ordering merged defects.
//!
//! Severity, then location. **Agreement is deliberately not used**, and that is
//! a reversal worth recording.
//!
//! It used to break ties within a severity band, on the reasoning that two
//! models finding the same defect is evidence it is real. The reasoning is
//! sound; the input is not there. Measured across a full multi-lane run of a
//! real repository and a narrow experiment designed to produce agreement — one
//! lane, three vendors, all pointed at the same three files — cross-vendor
//! agreement was 2 of 23 defects in the first and **0 of 7 pairs** in the
//! second. Independent judges confirmed the second.
//!
//! A tie-break that is almost always 1-versus-1 does not order anything; it
//! just implies a confidence signal exists. Ranking on it made the report claim
//! something the data does not support, so it is gone. Agreement is still
//! counted and still shown, because "two models saw this" is worth knowing when
//! it happens — it is simply not a rank.

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
    fn severity_decides_the_order() {
        let ranked = rank(vec![
            cluster_of(Severity::Low, 3, "a.rs", 1),
            cluster_of(Severity::Critical, 1, "b.rs", 1),
        ]);
        assert_eq!(ranked[0].cluster.severity(), Severity::Critical);
        assert_eq!(ranked[0].position, 1);
    }

    #[test]
    fn agreement_does_not_affect_the_order() {
        // Deliberate. Ranking on agreement implied a confidence signal that the
        // measurements do not support, so within a severity band the order is
        // decided by location alone and a widely-agreed defect gets no lift.
        let ranked = rank(vec![
            cluster_of(Severity::High, 1, "a.rs", 1),
            cluster_of(Severity::High, 3, "z.rs", 1),
        ]);
        assert_eq!(ranked[0].cluster.representative().anchor.file, "a.rs");
        assert_eq!(
            ranked[0].cluster.agreement, 1,
            "agreement pushed a defect up"
        );
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
