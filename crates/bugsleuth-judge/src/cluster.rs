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
/// Order-independent: the result is the connected components of "these two
/// describe the same defect", so any permutation of the input gives the same
/// grouping.
///
/// **The previous version claimed that and did not do it.** It joined a finding
/// to the *first* cluster it matched, reasoning that `same_defect` is symmetric.
/// Symmetry is not enough — the relation is not transitive, because wording
/// similarity is not. Given A matching B, and C matching both, `[A, B, C]`
/// produced one cluster while `[A, C, B]` could produce two, and the report a
/// user saw depended on the order sweeps happened to finish in. Merging every
/// cluster a finding matches makes the documented property true.
///
/// Found by BugSleuth reviewing itself, and it is the "comment describes
/// behaviour the code does not implement" class, in the doc comment of the
/// function whose stability the whole report rests on.
pub fn cluster(findings: Vec<Finding>) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();

    for finding in findings {
        let matching: Vec<usize> = clusters
            .iter()
            .enumerate()
            .filter(|(_, cluster)| {
                cluster
                    .findings
                    .iter()
                    .any(|other| same_defect(other, &finding))
            })
            .map(|(index, _)| index)
            .collect();

        match matching.split_first() {
            Some((&first, rest)) => {
                // Fold the others into the first, highest index out so the
                // remaining indices stay valid as they are removed.
                for &index in rest.iter().rev() {
                    let absorbed = clusters.remove(index);
                    clusters[first].findings.extend(absorbed.findings);
                }
                clusters[first].findings.push(finding);
            }
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
#[path = "cluster/tests.rs"]
mod tests;
