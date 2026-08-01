//! The part of the report an implementer actually works from.
//!
//! The ranked list answers "what is wrong and how bad". This answers "what do I
//! do about it", and it is written for a specific reader: a model running
//! locally on the user's own machine, given the repository and this text and
//! nothing else. It has not read the review. It cannot ask a question. It may
//! be a good deal weaker than whichever model found the defect.
//!
//! So the handoff repeats things the summary already said. That is deliberate —
//! each defect is a self-contained work order, because an implementer told to
//! fix defect 7 should not have to reconstruct defects 1 to 6 to understand it.

use bugsleuth_domain::Finding;
use bugsleuth_judge::Ranked;

/// Assemble the whole prompt: instructions, what was skipped, then the work.
///
/// Takes the pieces rather than a report type, because two different reports
/// carry them — a full `run` and a standalone `judge` — and the prompt handed to
/// an agent must be identical either way.
///
/// `not_reviewed` is not optional politeness. An agent given a list of defects
/// will assume it is the whole list unless it is told which parts of the
/// repository nobody looked at.
#[must_use]
pub fn prompt(repo: &str, ranked: &[Ranked], not_reviewed: &[String], sweeps: usize) -> String {
    let mut out = preamble(repo);

    if !not_reviewed.is_empty() {
        out.push_str("\n## What was NOT reviewed\n\n");
        for gap in not_reviewed {
            out.push_str(&format!("- {}\n", short_reason(gap)));
        }
        out.push_str(
            "\nNothing was looked for in those. Their absence from the list below \
             is not evidence that they are clean.\n",
        );
    }

    out.push_str(&format!(
        "\n## The defects, worst first ({})\n",
        ranked.len()
    ));
    for entry in ranked {
        out.push_str(&work_order(
            entry.position,
            entry.cluster.representative(),
            entry.cluster.agreement,
            sweeps,
        ));
    }
    out
}

/// One defect, written as a work order.
///
/// `position` is its rank in the merged report, so a person and a model reading
/// the same document are talking about the same number.
pub fn work_order(position: usize, finding: &Finding, agreement: usize, sources: usize) -> String {
    let mut out = String::new();
    let anchor = &finding.anchor;

    out.push_str(&format!(
        "\n### {position}. [{}] {}\n\n",
        finding.severity.as_str().to_uppercase(),
        finding.title
    ));
    out.push_str(&format!("- **Where:** `{}`:{}\n", anchor.file, anchor.line));
    // Reported, but no longer called confidence and no longer ranked on.
    // Measured cross-vendor agreement is near zero — 0 of 7 pairs in an
    // experiment built to produce it — so "1 of 7" says almost nothing about
    // whether a defect is real. It mostly says the other sweeps were reading
    // something else. Labelling that "confidence" invited exactly the wrong
    // inference.
    out.push_str(&format!(
        "- **Reported by:** {agreement} of {sources} sweeps that ran\n"
    ));
    if anchor.was_corrected() {
        // Worth saying: the model's own line number was wrong, so an implementer
        // searching for the quoted code should trust the corrected number.
        out.push_str(&format!(
            "- **Note:** the reviewer said line {}; the code was actually found at {}\n",
            anchor.claimed_line, anchor.line
        ));
    }
    out.push_str(&format!(
        "\n**The code as it stands** (`{}`):\n\n",
        anchor.file
    ));
    out.push_str("```\n");
    out.push_str(anchor.snippet.trim_end());
    out.push_str("\n```\n");
    out.push_str(&format!("\n**Why it is wrong:** {}\n", finding.explanation));
    out.push_str(&format!(
        "\n**How it fails:** {}\n",
        finding.failure_scenario
    ));

    let fix = &finding.fix;
    // A finding with no plan says so. Silence here would read as "no fix
    // needed", which is the opposite of what an empty plan means.
    if fix.approach.trim().is_empty() && fix.edits.is_empty() {
        out.push_str(
            "\n**Fix:** the reviewer returned no fix plan for this defect. \
             Diagnose it from the evidence above before changing anything.\n",
        );
        return out;
    }

    if !fix.approach.trim().is_empty() {
        out.push_str(&format!("\n**Approach:** {}\n", fix.approach));
    }
    if !fix.edits.is_empty() {
        out.push_str("\n**Edits, in order:**\n");
        for (n, edit) in fix.edits.iter().enumerate() {
            out.push_str(&format!(
                "\n{}. `{}` — {}\n\n{}\n",
                n + 1,
                edit.file,
                edit.location,
                indent(&edit.change)
            ));
        }
    }
    if !fix.verification.trim().is_empty() {
        out.push_str(&format!("\n**Prove it is fixed:** {}\n", fix.verification));
    }
    if !fix.risks.trim().is_empty() {
        out.push_str(&format!("\n**Watch out for:** {}\n", fix.risks));
    }
    out
}

/// The opening of the prompt, addressed to whoever is going to do the work.
///
/// Written as an instruction to a model, not as a description of a report,
/// because that is how it gets used: copied whole and pasted into a coding
/// agent pointed at the repository.
///
/// The scepticism is deliberate and is backed by measurement. Every location
/// here was checked to exist and the quoted code was found in it, but the
/// *reasoning* about that code was never executed. On a graded experiment
/// against known defects, one reviewer reported a fault in the same file and
/// the same function as a real bug, and it was a different bug — plausible,
/// well-argued and wrong. An implementer that treats this document as fact will
/// eventually "fix" something that was never broken.
pub fn preamble(repo: &str) -> String {
    format!(
        "You are fixing defects in the repository at `{repo}`.\n\n\
         Below is a list of defects found by an automated review. Each is a \
         self-contained work order: where it is, the code as it stands, why it \
         is wrong, how it fails, and what to change. Work them in the order \
         given — they are ordered by severity.\n\n\
         **Treat each one as a claim to verify, not a fact.** Every location was \
         checked to exist and the quoted code really is in that file, but no \
         reviewer ran the code. Some of these will be wrong in ways that read \
         convincingly.\n\n\
         For each defect, in order:\n\n\
         1. **Read the code first.** If it does not do what the finding says, \
         stop and say so. Do not edit it into agreement with the finding.\n\
         2. **Fix the root cause, not the line.** Where a fix names other \
         callers, change them too — a guard added to one caller and not its \
         siblings leaves the defect in.\n\
         3. **Leave the check behind.** Add the test described under \"Prove it \
         is fixed\" and run it. It must fail before your change and pass after. \
         A fix with no such test is a claim, not a fix.\n\
         4. **One defect per commit.** Do not roll several together; if one \
         turns out to be wrong, the rest should not be reverted with it.\n\n\
         When you are done, report for each defect: fixed, not a real defect \
         (with why), or could not fix (with what blocked you). Do not silently \
         skip any.\n"
    )
}

/// A gap line with the raw model output cut off it.
///
/// Failure reasons quote what the model actually said, which is the right thing
/// in the operator's report and the wrong thing here. One real reason carried a
/// truncated half-finished *finding* from a malformed reply — and this document
/// is read by an agent that is being handed findings to act on. Text that looks
/// like an instruction must not arrive by accident.
///
/// The operator still sees the whole reason; only the prompt is trimmed.
fn short_reason(gap: &str) -> String {
    const CUTS: [&str; 3] = ["; reply began", "; it began", "; the reply began"];
    let mut trimmed = gap;
    for cut in CUTS {
        if let Some(at) = trimmed.find(cut) {
            trimmed = &trimmed[..at];
        }
    }
    // A backstop for reasons that quote without one of those markers.
    const MAX: usize = 160;
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let short: String = trimmed.chars().take(MAX).collect();
    format!("{short}…")
}

/// Indent a block so it renders as a fenced code chunk inside a list item.
fn indent(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("   {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::{FindingId, LaneId, ModelId, Severity, VerifiedAnchor};

    fn finding(fix: bugsleuth_domain::FixPlan) -> Finding {
        Finding {
            id: FindingId::new("f1"),
            lane: LaneId::new("Correctness"),
            model: ModelId::new("claude:sonnet"),
            title: "Divides by zero on an empty inventory".into(),
            severity: Severity::High,
            anchor: VerifiedAnchor {
                file: "src/price.rs".into(),
                line: 42,
                claimed_line: 40,
                snippet: "let avg = total / items.len();".into(),
            },
            explanation: "No check for an empty inventory.".into(),
            failure_scenario: "An empty inventory panics.".into(),
            fix,
        }
    }

    #[test]
    fn a_defect_with_no_fix_plan_says_so_rather_than_going_quiet() {
        // Silence would read as "nothing to do here", which is the opposite of
        // what a missing plan means — and this is the case that happens whenever
        // an older sweep report is merged.
        let text = work_order(1, &finding(Default::default()), 1, 3);
        assert!(text.contains("no fix plan"), "{text}");
    }

    #[test]
    fn a_corrected_line_number_is_called_out_so_the_right_one_is_used() {
        // The implementer will go to a line. If the reviewer's number was wrong
        // and we show both without comment, they may pick the wrong one.
        let text = work_order(1, &finding(Default::default()), 1, 3);
        assert!(text.contains("said line 40"), "{text}");
        assert!(text.contains("actually found at 42"), "{text}");
    }

    #[test]
    fn the_work_order_carries_every_part_of_the_plan() {
        let plan = bugsleuth_domain::FixPlan {
            approach: "Guard the empty case".into(),
            edits: vec![bugsleuth_domain::FixEdit {
                file: "src/price.rs".into(),
                location: "fn average_price".into(),
                change: "if items.is_empty() { return 0; }".into(),
            }],
            verification: "cargo test average_price_of_empty".into(),
            risks: "checked both callers".into(),
        };
        let text = work_order(3, &finding(plan), 2, 3);
        for expected in [
            "Guard the empty case",
            "fn average_price",
            "items.is_empty()",
            "cargo test average_price_of_empty",
            "checked both callers",
            "2 of 3 sweeps",
            "src/price.rs",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn a_gap_line_does_not_smuggle_another_models_findings_into_the_prompt() {
        // Real reason from a run. Kilo answered with malformed JSON, and the
        // failure reason quoted it — including a half-written finding. This
        // document is handed to an agent that acts on findings, so text that
        // reads like one must not arrive by accident.
        let gap = "Correctness lane, by kilo — the model's reply did not match the required                    structure: missing field `title`; reply began \"{\\\"findings\\\":                   [{\\\"category\\\":\\\"correctness\\\",\\\"description\\\":                   \\\"All-day event window overlap is checked with lexicographic string";
        let line = short_reason(gap);
        assert!(
            !line.contains("findings"),
            "raw model output survived: {line}"
        );
        assert!(!line.contains("All-day event"), "a finding leaked: {line}");
        assert!(
            line.contains("missing field"),
            "the useful part was lost: {line}"
        );
    }

    #[test]
    fn a_gap_with_no_quoted_output_is_left_alone() {
        let gap = "Security lane, by claude:sonnet — the claude CLI exited with code 1";
        assert_eq!(short_reason(gap), gap);
    }
}
