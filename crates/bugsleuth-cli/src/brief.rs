//! Assembling the review brief a lane sweep is given.
//!
//! The brief is the highest-leverage text in the tool. Two things it must do
//! that are easy to get wrong: keep the model inside its lane's mandate, and
//! make it understand that quoting code inaccurately silently destroys its own
//! finding. The second is stated bluntly and twice, because the anchor check
//! discards non-matching quotes without argument and a model that does not know
//! that will produce paraphrased snippets and lose real defects.

use bugsleuth_domain::Lane;

/// Build the full brief for one lane sweep.
pub fn build(lane: Lane, scope: Option<&str>) -> String {
    let scope_line = match scope {
        Some(paths) if !paths.trim().is_empty() => {
            format!("\nRestrict your review to these paths: {}\n", paths.trim())
        }
        _ => String::new(),
    };

    format!(
        "\
You are performing an adversarial code review of the repository in your current \
working directory. You are one of several independent reviewers, each with a \
different mandate. Stay strictly inside yours.

# Your mandate: the {title} lane

{mandate}
{scope_line}
# How to work

Read the code. Navigate the repository yourself — use Grep and Glob to find the \
files that matter for your mandate, then read them properly. Do not guess at \
what a file contains.

Report only defects you can point at in the code. This review is read for \
someone who cannot check your work by reading the source, so an confident \
wrong finding is worse than no finding: it costs them trust in every other \
finding in the report. If you are unsure whether something is really a defect, \
leave it out.

# Quoting code: this determines whether your finding survives

Every finding you report is checked automatically. The `snippet` field is \
searched for in the file named by `file`. If the code you quote does not appear \
in that file, the finding is DISCARDED without review — no matter how real the \
defect is.

So: copy the offending line or lines EXACTLY as they appear in the file, \
character for character, from what you actually read. Do not retype from \
memory, do not paraphrase, do not tidy the code up, do not add or remove \
comments. Indentation is forgiven; everything else is not.

Keep the snippet short — the one to five lines that actually contain the \
defect, not the whole function.

The `line` number should be where that snippet starts, but a small error there \
is tolerated and corrected automatically. An inaccurate snippet is not.

# Output

Return your findings in the required JSON structure. Report an empty list if \
you found nothing that meets your mandate. Never invent a finding to avoid \
returning an empty list — an empty result is a valid and useful answer.",
        title = lane.title(),
        mandate = lane.mandate(),
        scope_line = scope_line,
    )
}

/// Build the brief for a proof attempt.
///
/// The instruction not to fix the defect is the load-bearing one. Asked to make
/// a test fail, a coding agent's strongest instinct is to make the code correct
/// — which produces a passing test that proves nothing, and it will do it
/// without being asked.
pub fn proof(defect: &str, test_command: &str) -> String {
    format!(
        "\
You are in a throwaway checkout of a repository that contains a known defect. \
Your job is to prove that the defect is real by writing a test that FAILS \
because of it.

# The defect

{defect}

# What to do

1. Read the code and confirm you understand the defect.
2. Add ONE new test, in the same style as the tests already in this repository, \
that fails specifically because of this defect.
3. Run it with `{test_command}` and confirm you see it fail.
4. Report the test's name.

# What NOT to do

DO NOT FIX THE DEFECT. Do not change any production code at all. Your only \
change should be the added test. This is checked automatically: if any \
previously passing test stops passing, your proof is rejected, because a test \
that fails only because you altered the code proves nothing about the original \
defect.

Do not weaken, delete or modify any existing test.

Do not write a test that fails for an unrelated reason — it must fail because \
of THIS defect, and it must pass once this defect is fixed.

# If you cannot

If you cannot write a test that fails because of this defect, say so and \
explain precisely why, leaving `wrote_failing_test` false. That is a genuinely \
useful answer: a defect nobody can demonstrate is a defect worth doubting. It \
is far better than a test that fails for the wrong reason.

# Report

Return the required JSON structure. `test_name` must be the exact filter that \
selects only your new test — letters, digits, underscores and `::` only.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proof_brief_forbids_fixing_the_defect() {
        let text = proof("something is wrong", "cargo test");
        assert!(text.contains("DO NOT FIX THE DEFECT"));
        assert!(text.contains("cargo test"));
        assert!(text.contains("something is wrong"));
    }

    #[test]
    fn each_lane_gets_its_own_mandate_and_not_another_lanes() {
        let ux = build(Lane::Ux, None);
        assert!(ux.contains("Aesthetic opinions"));
        assert!(!ux.contains("SQL injection"));

        let security = build(Lane::Security, None);
        assert!(security.contains("authorization"));
        assert!(!security.contains("Aesthetic opinions"));
    }

    #[test]
    fn the_brief_always_explains_that_a_bad_quote_discards_the_finding() {
        for lane in Lane::ALL {
            assert!(
                build(lane, None).contains("DISCARDED"),
                "{lane} brief omitted the anchor warning"
            );
        }
    }

    #[test]
    fn a_path_scope_appears_only_when_one_was_given() {
        assert!(!build(Lane::Correctness, None).contains("Restrict your review"));
        assert!(!build(Lane::Correctness, Some("  ")).contains("Restrict your review"));
        assert!(build(Lane::Correctness, Some("src/")).contains("Restrict your review"));
    }
}
