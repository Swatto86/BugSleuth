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

    let prompt = prompt_for(clusters, Some(request.repo));
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

    let salvaged = verdicts.salvaged;
    let by_id: BTreeMap<String, (Severity, String)> = verdicts
        .verdicts
        .verdicts
        .into_iter()
        .map(|v| (v.id, (v.severity, v.reason)))
        .collect();

    let mut changed = 0;
    let mut graded = 0;
    for (index, cluster) in clusters.iter_mut().enumerate() {
        // A verdict for an id that was never offered is dropped by construction:
        // only offered ids are looked up.
        let Some((severity, reason)) = by_id.get(&id_of(index)) else {
            continue;
        };
        graded += 1;
        if *severity != cluster.claimed_severity() {
            changed += 1;
        }
        cluster.triaged = Some(*severity);

        // A dismissal carries its own proof, exactly as a finding does. The
        // model may say the code already documents this, but only a quote
        // actually present in the file settles it — otherwise the easiest way
        // to make a report look clean would be to claim every defect was
        // already known.
        match acknowledgement(reason, request.repo, cluster) {
            Some((quote, rest)) => {
                cluster.acknowledged = Some(quote);
                cluster.triage_reason = Some(rest);
            }
            None => cluster.triage_reason = Some(reason.clone()),
        }
    }

    Outcome {
        changed,
        graded,
        note: grading_note(salvaged, graded, clusters.len()),
    }
}

/// The quote a verdict offers as proof the code already acknowledges this, if
/// that quote is really in the file it names.
///
/// Returns the quote and the rest of the reason. `None` when the verdict makes
/// no such claim, when the file cannot be read, or — the case that matters —
/// when the quoted words are not there. An unfounded dismissal is discarded and
/// the finding stands, which is the safe direction: the cost of being wrong is
/// a reader seeing a defect they already knew about, against a reader never
/// seeing a real one.
fn acknowledgement(reason: &str, repo: &Path, cluster: &Cluster) -> Option<(String, String)> {
    const MARKER: &str = "ACKNOWLEDGED:";
    let rest = reason.trim().strip_prefix(MARKER)?.trim();

    // The quote is whatever the model put first, up to the end of a sentence or
    // the line. Compared with whitespace collapsed, since a comment is wrapped
    // across lines with leading slashes and the model will not reproduce that.
    let quote = rest
        .trim_start_matches(['"', '\''])
        .split(['"', '\n'])
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.')
        .to_string();
    if quote.split_whitespace().count() < 4 {
        // Too short to be evidence of anything.
        return None;
    }

    let file = repo.join(&cluster.representative().anchor.file);
    let source = std::fs::read_to_string(file).ok()?;
    let flatten = |text: &str| {
        text.split_whitespace()
            .map(|word| word.trim_matches(|c: char| c == '/' || c == '*'))
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    if !flatten(&source).contains(&flatten(&quote)) {
        return None;
    }
    Some((quote, rest.to_string()))
}

/// The id a defect is offered under. Its position, which is stable for the
/// length of one call and means nothing outside it.
fn id_of(index: usize) -> String {
    (index + 1).to_string()
}

/// How the report describes a grading pass that covered `graded` of `total`
/// defects: recovered after a cutoff, a clean full pass (empty note), or a
/// partial pass. One place so the salvaged wording has exactly one definition.
fn grading_note(salvaged: bool, graded: usize, total: usize) -> String {
    if salvaged {
        format!(
            "the triage pass ran out of turns and its grades were recovered afterwards, so {graded} of {total} defects were compared rather than all of them"
        )
    } else if graded == total {
        String::new()
    } else {
        let missing = total - graded;
        format!(
            "the triage pass graded {graded} of {total} defects; the other {missing} keep \
             the grade the model that found them gave"
        )
    }
}

/// The defect list, as the grading model sees it.
///
/// Each entry carries what it would take a person to judge consequence: where
/// the defect is, what goes wrong, and what the finder called it. The current
/// grade is included deliberately — withholding it would not produce an
/// independent opinion, it would produce one made with less information.
pub(crate) fn prompt_for(clusters: &[Cluster], repo: Option<&Path>) -> String {
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
         what goes wrong, and why it is wrong. Answer in one reply.\n\n\
         Some entries carry \"What the code says about itself here\": the comment written at \
         that exact location. Read it. If it shows the concern is a deliberate, recorded \
         decision rather than an oversight — the trade-off named, a reason given, an \
         alternative rejected — then begin your reason with `ACKNOWLEDGED: ` and a short \
         verbatim quote from that comment, copied exactly. Quote only what is really there: \
         a quote that cannot be found in the file is discarded, and the dismissal with it. A \
         comment that merely restates what the code does is not an acknowledgement. Grade \
         such an entry as you otherwise would — the marker changes where it is reported, not \
         what it is worth.\n\n",
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
        if let Some(said) = repo.and_then(|root| commentary_at(root, finding)) {
            out.push_str(&format!(
                "What the code says about itself here:
{said}

"
            ));
        }
    }
    out
}

/// The comment block written immediately above a finding's anchor.
///
/// Read because a reviewer is not the only one who has looked at this code: the
/// person who wrote it may have already recorded why it is the way it is. Four
/// of the five findings that failed scrutiny across two self-reviews were
/// accurate observations about deliberate trade-offs documented at the exact
/// line named — and the words settling them were sitting in the file the whole
/// time, unread.
///
/// Contiguous comment lines only, and a bounded number of them: this goes into
/// a prompt, and a run of commented-out code above a defect is not commentary
/// about it.
fn commentary_at(repo: &Path, finding: &bugsleuth_domain::Finding) -> Option<String> {
    const MOST: usize = 24;
    let text = std::fs::read_to_string(repo.join(&finding.anchor.file)).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    // Anchors are 1-based, and the comment sits above the line itself.
    let mut index = usize::try_from(finding.anchor.line).ok()?.checked_sub(1)?;

    let mut block: Vec<&str> = Vec::new();
    while index > 0 && block.len() < MOST {
        index -= 1;
        let line = lines.get(index)?.trim();
        let is_comment = line.starts_with("//") || line.starts_with('#') || line.starts_with('*');
        if !is_comment {
            // Attributes and blank lines sit between a doc comment and the item
            // it documents, so they do not end the block.
            if line.is_empty() || line.starts_with("#[") {
                continue;
            }
            break;
        }
        block.push(line);
    }
    if block.is_empty() {
        return None;
    }
    block.reverse();
    Some(block.join(
        "
",
    ))
}

#[cfg(test)]
#[path = "triage/tests.rs"]
mod tests;
