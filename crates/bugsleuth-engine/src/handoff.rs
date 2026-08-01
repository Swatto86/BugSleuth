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
    out.push_str(&format!(
        "- **Confidence:** found independently by {agreement} of {sources} sweeps\n"
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

/// The standing instructions at the top of the handoff.
///
/// Two of these exist because of how the findings were produced: a defect only
/// one sweep saw is less certain than one three saw, and a reviewer that never
/// ran the code can still be wrong about it. An implementer that treats every
/// line here as fact will eventually "fix" something that was never broken.
pub fn preamble() -> &'static str {
    "\n## How to use this section\n\n\
     Each defect below is a self-contained work order: location, the code as it \
     stands, why it is wrong, and what to change. Work them in order — they are \
     ranked by severity and by how many independent reviewers found them.\n\n\
     Before you change anything:\n\n\
     1. **Read the code first.** Every location was checked to exist and the \
     quoted code was found there, but the reasoning about it was not executed. \
     If the code does not do what the finding says, stop and report that \
     instead of editing.\n\
     2. **Fix the root cause, not the line.** Where a fix names other callers, \
     change them too; a guard added to one caller and not its siblings leaves \
     the defect in.\n\
     3. **Leave the check behind.** Add the test named under \"Prove it is \
     fixed\" and run it. A fix with no failing-then-passing test is a claim.\n\
     4. **One defect per change.** Do not roll several together; if one is \
     wrong, all of them get reverted.\n"
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
}
