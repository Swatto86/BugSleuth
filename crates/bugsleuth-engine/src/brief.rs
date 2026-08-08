//! Assembling the review brief a lane sweep is given.
//!
//! The brief is the highest-leverage text in the tool. Two things it must do
//! that are easy to get wrong: keep the model inside its lane's mandate, and
//! make it understand that quoting code inaccurately silently destroys its own
//! finding. The second is stated bluntly and twice, because the anchor check
//! discards non-matching quotes without argument and a model that does not know
//! that will produce paraphrased snippets and lose real defects.

use bugsleuth_domain::{Lane, finding_schema};

/// Build the full brief for one lane sweep.
///
/// `enforced_schema` says whether the CLI itself will constrain the reply to
/// our JSON Schema. Claude and Codex can; Kilo cannot, so for Kilo the schema
/// has to be spelled out in the prompt instead. That is strictly weaker — a
/// prompt is a request, a schema is a constraint — so expect more malformed
/// replies from a vendor that needs the appendix.
pub fn build(lane: Lane, scope: Option<&str>, enforced_schema: bool) -> String {
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

# The fix: who you are writing it for

Every finding carries a `fix`. It will be handed to a **small model running on \
the user's own machine**, which has not read your review, cannot ask you \
anything, and is not as strong a reasoner as you are. It has the repository and \
your instructions, and nothing else.

Write for that reader:

- Name the root cause in `approach`, not the symptom. If the same mistake \
appears in several places, say where — a fix applied to one caller and not its \
siblings leaves the bug in.
- In `edits`, give the actual replacement code, not a description of it. Say \
which function or symbol to edit, so the instruction survives the line numbers \
moving.
- In `verification`, give a test that fails before the change and passes after, \
and the exact command that runs it. Match the project's existing test \
conventions — look at a test file before you describe one.
- In `risks`, name the other callers you checked. Write \"none identified\" only \
if you actually looked.

A fix a competent implementer cannot follow without rediscovering your \
reasoning is not finished.

# Output

Return your findings in the required JSON structure. Report an empty list if \
you found nothing that meets your mandate. Never invent a finding to avoid \
returning an empty list — an empty result is a valid and useful answer.{schema_appendix}",
        title = lane.title(),
        mandate = lane.mandate(),
        scope_line = scope_line,
        schema_appendix = if enforced_schema {
            String::new()
        } else {
            schema_appendix()
        },
    )
}

/// Spell out the required JSON shape for a CLI that cannot enforce one.
///
/// The instruction to emit nothing but the object matters as much as the schema
/// itself. Without a constraint, the natural thing for a model to do is
/// introduce its answer with a sentence of prose — and while the parser
/// tolerates that, it is one more way for a paid-for sweep to come back
/// unreadable.
fn schema_appendix() -> String {
    format!(
        "

Your reply must be a single JSON object and NOTHING else — no prose before or after it, and no code fence. It must validate against this JSON Schema:

{}

Here is a complete, correctly-shaped reply with one finding in it. Copy this structure exactly, including every field name:

{}

Use these field names and no others. A reply that is not valid JSON matching the schema is discarded entirely, however good the findings in it are — and the single most common way that happens is inventing a field name, so check yours against the example before you answer.",
        serde_json::to_string_pretty(&finding_schema()).unwrap_or_default(),
        EXAMPLE_REPLY,
    )
}

/// A worked example of a correct reply, for the vendor that cannot be given a
/// schema to enforce.
///
/// Not decoration. Kilo's first sweep of a real repository under the current
/// schema came back with `category` where `title` belongs and was discarded
/// whole — a schema tells a model what is allowed, an example shows it what to
/// write, and only one of those survives a long specification. The fix plan
/// made the schema noticeably longer, which made this worse.
///
/// Kept in sync by `the_example_reply_is_actually_valid`, which parses it with
/// the same type a real reply is parsed with.
const EXAMPLE_REPLY: &str = r#"{
  "findings": [
    {
      "title": "average_price divides by zero on an empty inventory",
      "severity": "high",
      "file": "src/inventory.rs",
      "line": 42,
      "snippet": "let average = total / items.len() as u32;",
      "explanation": "items.len() is zero for an empty inventory, so this divides by zero.",
      "failure_scenario": "Call average_price() on a new Inventory. It panics in debug builds.",
      "fix": {
        "approach": "Return None for an empty inventory rather than dividing. The root cause is a missing empty check, and the same pattern appears in median_price().",
        "edits": [
          {
            "file": "src/inventory.rs",
            "location": "fn average_price",
            "change": "Replace the body with:\n\nif items.is_empty() { return None; }\nSome(total / items.len() as u32)"
          }
        ],
        "verification": "Add a test asserting average_price() returns None for an empty Inventory, then run `cargo test average_price`. It fails before the change and passes after.",
        "risks": "Callers now receive Option. I checked both call sites in src/report.rs; each already handles a missing price."
      }
    }
  ]
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_lane_gets_its_own_mandate_and_not_another_lanes() {
        let ux = build(Lane::Ux, None, true);
        assert!(ux.contains("Aesthetic opinions"));
        assert!(!ux.contains("SQL injection"));

        let security = build(Lane::Security, None, true);
        assert!(security.contains("authorization"));
        assert!(!security.contains("Aesthetic opinions"));
    }

    #[test]
    fn the_brief_always_explains_that_a_bad_quote_discards_the_finding() {
        for lane in Lane::ALL {
            assert!(
                build(lane, None, true).contains("DISCARDED"),
                "{lane} brief omitted the anchor warning"
            );
        }
    }

    #[test]
    fn a_vendor_without_schema_enforcement_gets_the_schema_in_the_prompt() {
        let enforced = build(Lane::Correctness, None, true);
        let described = build(Lane::Correctness, None, false);
        assert!(!enforced.contains("JSON Schema"));
        assert!(described.contains("JSON Schema"));
        assert!(
            described.contains("failure_scenario"),
            "the schema itself must be present"
        );
    }

    #[test]
    fn a_path_scope_appears_only_when_one_was_given() {
        assert!(!build(Lane::Correctness, None, true).contains("Restrict your review"));
        assert!(!build(Lane::Correctness, Some("  "), true).contains("Restrict your review"));
        assert!(build(Lane::Correctness, Some("src/"), true).contains("Restrict your review"));
    }

    #[test]
    fn the_example_reply_is_actually_valid() {
        // The example is what the weakest vendor copies, so an example that did
        // not parse would teach it to produce replies we then discard. Parsed
        // with the same type a real reply is parsed with, so it cannot drift
        // from the schema without failing here.
        let parsed: bugsleuth_domain::RawFindings = serde_json::from_str(EXAMPLE_REPLY)
            .unwrap_or_else(|e| panic!("the example reply does not parse: {e}"));
        assert_eq!(parsed.findings.len(), 1);
        let finding = &parsed.findings[0];
        assert!(!finding.title.is_empty());
        assert!(!finding.snippet.is_empty());
        // Every part of the fix plan is exercised, or the example would be
        // showing an incomplete shape as if it were complete.
        assert!(!finding.fix.approach.is_empty());
        assert_eq!(finding.fix.edits.len(), 1);
        assert!(!finding.fix.edits[0].location.is_empty());
        assert!(!finding.fix.verification.is_empty());
        assert!(!finding.fix.risks.is_empty());
    }

    #[test]
    fn only_the_vendor_that_needs_the_schema_is_given_it() {
        // The appendix is long. Sending it to a vendor whose CLI already
        // enforces the schema is wasted context in every single sweep.
        let enforced = build(Lane::Correctness, None, true);
        let spelled_out = build(Lane::Correctness, None, false);
        assert!(
            !enforced.contains("EXAMPLE"),
            "example leaked into the enforced brief"
        );
        assert!(!enforced.contains("average_price divides by zero"));
        assert!(spelled_out.contains("average_price divides by zero"));
        assert!(spelled_out.len() > enforced.len());
    }
}
