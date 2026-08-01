//! Grading every defect in a report against the others.
//!
//! Severity is the only thing that orders a BugSleuth report, and it was
//! self-assigned: each model graded its own findings, in isolation, with no
//! view of anything else the run turned up. Graded by hand against a real
//! multi-lane run, 6 of 14 severities were wrong, in both directions.
//!
//! One pass fixes the structural half of that. "Worst first" is a claim about a
//! total order, and a total order comes from comparison — so the whole merged
//! list goes to one model at once, with the same rubric the sweeps were given.
//!
//! Three rules this pass obeys, all of them about not making the report worse:
//! it may not add, remove, merge or re-word a defect; a verdict for a defect
//! that was never offered is discarded rather than guessed at; and if the pass
//! fails for any reason the models' own grades stand and the report says the
//! pass did not run. A tool whose reader cannot check the code must never
//! quietly reorder itself on the strength of a call that did not happen.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use bugsleuth_domain::Severity;
use bugsleuth_judge::Cluster;
use bugsleuth_provider::claude::{TriageRequest, triage as claude_triage};

/// One answer, no exploration.
///
/// **Measured, twice.** With the repository readable, the pass spent its entire
/// budget reading files and returned nothing at all — at 12 turns and again at
/// 30. It grades from the descriptions now, which is a turn's work, and the cap
/// exists only so a model that starts rambling is stopped rather than billed.
const MAX_TURNS: u32 = 12;

/// How a report describes severities that no pass ever compared. Written once
/// because the two ways of getting there — no model configured, and a pass that
/// failed — must not read as two different states of the report.
const UNGRADED: &str = "severities are each model's own assessment of its own finding, ungraded";

pub struct Request<'a> {
    pub repo: &'a Path,
    /// Which model grades. A cheap one is the point: this is a comparison over
    /// summaries, not a second review.
    pub model: &'a str,
    pub effort: &'a str,
    pub timeout: Duration,
    pub api_key: Option<&'a str>,
}

/// What the pass did, in the words the report will use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Defects whose severity the pass changed.
    pub changed: usize,
    /// Defects it graded at all.
    pub graded: usize,
    /// Why the pass did not run, or did not cover everything. Empty when the
    /// whole list was graded.
    pub note: String,
}

/// Re-grade a merged report, or say why it was not.
///
/// Separate from [`apply`] so a run that turned the pass off still produces the
/// same kind of statement about how its severities were decided as one where
/// the pass failed. Silence would read as "graded".
pub(crate) async fn grade(
    clusters: &mut [Cluster],
    options: &crate::orchestrate::RunOptions<'_>,
) -> Outcome {
    if options.triage_model.trim().is_empty() {
        return Outcome {
            note: UNGRADED.to_string() + ": no triage model is configured",
            ..Outcome::default()
        };
    }
    apply(
        clusters,
        Request {
            repo: options.repo,
            model: options.triage_model,
            effort: "",
            timeout: options.timeout,
            api_key: options.api_key,
        },
    )
    .await
}

/// Re-grade a merged report. Returns the clusters either way — a failed triage
/// pass changes nothing except what the report says about itself.
pub async fn apply(clusters: &mut [Cluster], request: Request<'_>) -> Outcome {
    if clusters.len() < 2 {
        // Comparison is the entire mechanism. One defect has nothing to be
        // graded against, and paying for a call that can only restate one
        // model's own opinion is waste.
        return Outcome::default();
    }

    let prompt = prompt_for(clusters);
    let verdicts = match claude_triage(TriageRequest {
        repo: request.repo,
        model: request.model,
        effort: request.effort,
        prompt: &prompt,
        timeout: request.timeout,
        max_turns: MAX_TURNS,
        binary: None,
        api_key: request.api_key,
    })
    .await
    {
        Ok(verdicts) => verdicts,
        Err(error) => {
            return Outcome {
                note: format!(
                    "severities are each model's own assessment of its own finding, \
                     ungraded: the triage pass failed ({error})"
                ),
                ..Outcome::default()
            };
        }
    };

    let by_id: BTreeMap<String, (Severity, String)> = verdicts
        .verdicts
        .into_iter()
        .map(|v| (v.id, (v.severity, v.reason)))
        .collect();

    let mut changed = 0;
    let mut graded = 0;
    for (index, cluster) in clusters.iter_mut().enumerate() {
        // A verdict for an id that was never offered is dropped by construction:
        // only offered ids are looked up.
        let Some((severity, _)) = by_id.get(&id_of(index)) else {
            continue;
        };
        graded += 1;
        if *severity != cluster.claimed_severity() {
            changed += 1;
        }
        cluster.triaged = Some(*severity);
    }

    let missing = clusters.len() - graded;
    Outcome {
        changed,
        graded,
        note: if missing == 0 {
            String::new()
        } else {
            // Named rather than silently left at the model's own grade: a
            // partial re-grade means the list is ordered by two different
            // standards at once, which is worth knowing.
            format!(
                "the triage pass graded {graded} of {} defects; the other {missing} keep \
                 the grade the model that found them gave",
                clusters.len()
            )
        },
    }
}

/// The id a defect is offered under. Its position, which is stable for the
/// length of one call and means nothing outside it.
fn id_of(index: usize) -> String {
    (index + 1).to_string()
}

/// The defect list, as the grading model sees it.
///
/// Each entry carries what it would take a person to judge consequence: where
/// the defect is, what goes wrong, and what the finder called it. The current
/// grade is included deliberately — withholding it would not produce an
/// independent opinion, it would produce one made with less information.
pub(crate) fn prompt_for(clusters: &[Cluster]) -> String {
    let mut out = String::new();
    out.push_str(
        "You are grading the severity of defects already found in this repository by other \
         reviewers. Every one has been checked to exist: the code each quotes was located in \
         the file it names. Your job is ordering, not review.\n\n\
         Grade all of them together, against each other. Each was graded in isolation by \
         whoever found it, with no view of the rest of this list, which is exactly what makes \
         those grades unreliable — a reviewer who found nothing worse than an edge case will \
         still have called one of them high.\n\n\
         Do not add defects. Do not remove any. Do not merge or re-word them. Return exactly \
         one verdict per id below, and keep the ids as they are written.\n\n\
         Grade from what is written below. You have no tools and nothing to open: each entry \
         was written by a reviewer who did read the code, and carries where the defect is, \
         what goes wrong, and why it is wrong. Answer in one reply.\n\n",
    );

    for (index, cluster) in clusters.iter().enumerate() {
        let finding = cluster.representative();
        out.push_str(&format!(
            "## id {}\n\
             Title: {}\n\
             Location: {}:{}\n\
             Currently graded: {} (by the reviewer that found it)\n\
             Reported by: {} model(s)\n\
             What goes wrong: {}\n\
             Why the code is wrong: {}\n\n",
            id_of(index),
            finding.title,
            finding.anchor.file,
            finding.anchor.line,
            finding.severity.as_str(),
            cluster.agreement.max(1),
            finding.failure_scenario,
            finding.explanation,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::{Finding, FindingId, Lane, ModelId, RawFinding, VerifiedAnchor};

    fn cluster_at(severity: Severity, title: &str) -> Cluster {
        Cluster {
            findings: vec![Finding::new(
                FindingId::new("f"),
                Lane::Correctness,
                ModelId::new("claude:sonnet"),
                RawFinding {
                    title: title.into(),
                    severity,
                    file: "a.rs".into(),
                    line: 1,
                    snippet: "x".into(),
                    explanation: "e".into(),
                    failure_scenario: "boom".into(),
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
        }
    }

    #[test]
    fn every_defect_is_offered_under_an_id_the_reply_can_name() {
        let clusters = vec![
            cluster_at(Severity::Low, "first"),
            cluster_at(Severity::High, "second"),
        ];
        let prompt = prompt_for(&clusters);
        assert!(prompt.contains("## id 1"));
        assert!(prompt.contains("## id 2"));
        assert!(prompt.contains("first"));
        assert!(prompt.contains("second"));
    }

    #[test]
    fn the_prompt_says_what_each_defect_was_already_graded() {
        // Withholding it would not buy independence, only a judgement made with
        // less to go on.
        let prompt = prompt_for(&[cluster_at(Severity::Critical, "t")]);
        assert!(prompt.contains("Currently graded: critical"), "{prompt}");
    }

    #[test]
    fn the_prompt_forbids_the_three_things_that_would_damage_the_report() {
        let prompt = prompt_for(&[cluster_at(Severity::Low, "t")]);
        for rule in ["Do not add defects", "Do not remove", "Do not merge"] {
            assert!(prompt.contains(rule), "the prompt does not say: {rule}");
        }
    }

    #[tokio::test]
    async fn a_single_defect_is_not_paid_to_be_compared_with_nothing() {
        // Comparison is the whole mechanism; with one defect there is nothing to
        // compare and the call could only restate the model's own opinion.
        let mut clusters = vec![cluster_at(Severity::Low, "only")];
        let outcome = apply(
            &mut clusters,
            Request {
                repo: Path::new("."),
                // A binary that does not exist: if this ever tried to run, the
                // test would fail rather than quietly spending quota.
                model: "no-such-model",
                effort: "",
                timeout: Duration::from_secs(1),
                api_key: None,
            },
        )
        .await;
        assert_eq!(outcome, Outcome::default());
        assert_eq!(clusters[0].triaged, None);
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn the_prompt_never_offers_tools_the_pass_does_not_have() {
        // It once did, for a whole afternoon of runs. Told to read files it had
        // no way to open, the pass burned every turn and returned nothing at all
        // — and the failure looked exactly like a turn limit set too low.
        let prompt = prompt_for(&[]);
        for offer in ["You may read", "open the file", "Read only what you need"] {
            assert!(!prompt.contains(offer), "the prompt still offers: {offer}");
        }
        assert!(prompt.contains("no tools"), "{prompt}");
    }
}
