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

#[cfg(test)]
mod tests {
    use super::*;

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
