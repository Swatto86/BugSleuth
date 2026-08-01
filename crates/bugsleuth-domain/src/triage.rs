//! Re-grading severity across a whole report.
//!
//! Severity is the only thing that orders a BugSleuth report, and until now it
//! was whatever each model happened to call its own finding. Graded by hand
//! against a real run, 6 of 14 severities were wrong — in both directions, so
//! not a bias that could be corrected with an offset.
//!
//! The reason is structural rather than a failure of any one model. A sweep
//! grades each defect it finds in isolation, with no reference class: a lane
//! that turns up nothing worse than an unhandled edge case will still crown one
//! of them "high", because nothing else is there to be worse than. But "worst
//! first" is a claim about a *total order*, and a total order can only come from
//! comparison. So one pass looks at every merged defect together and grades them
//! against each other and against a written rubric.
//!
//! This never invents, removes, or re-words a defect. It assigns one enum value
//! per defect and says why.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::finding::Severity;

/// How severity is decided, in one place because two prompts ask for it.
///
/// Used both by the sweep schema, where a model grades what it just found, and
/// by the triage pass that re-grades everything together. They must not drift:
/// a triage pass applying a different standard from the one the sweep was given
/// would look like disagreement between models when it is really disagreement
/// between two prompts.
pub const SEVERITY_RUBRIC: &str = "Judge by the consequence to a user, NOT by \
how interesting the bug is or how hard it was to find. critical: data loss, a \
security breach, or the application unusable. high: a common action fails or \
misleads the user, or an accessibility barrier locks someone out of an outcome \
entirely. medium: a real problem that has a workaround, or one that only \
affects an uncommon path. low: minor, cosmetic, or very rare. Ask what happens \
to the person using this, and whether they have another way to get what they \
wanted.";

/// One defect's severity, decided with every other defect in view.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SeverityVerdict {
    /// The `id` the defect was listed under. Matched back by exact string: a
    /// verdict whose id was never offered is dropped rather than guessed at.
    pub id: String,
    pub severity: Severity,
    /// One sentence naming the consequence that decided it. Shown in the report
    /// so a re-grade is never a silent contradiction of what a model said.
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SeverityVerdicts {
    pub verdicts: Vec<SeverityVerdict>,
}

/// JSON Schema for a triage reply.
pub fn triage_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["verdicts"],
        "properties": {
            "verdicts": {
                "type": "array",
                "description": "Exactly one entry per defect you were given, using the id shown. Do not add defects, do not omit defects, and do not merge them.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "severity", "reason"],
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The id of the defect you are grading, copied exactly."
                        },
                        "severity": {
                            "type": "string",
                            "enum": ["critical", "high", "medium", "low"],
                            "description": SEVERITY_RUBRIC
                        },
                        "reason": {
                            "type": "string",
                            "description": "One sentence naming the consequence to a user that decides this grade. If you are moving it from what the finding claimed, say what the finding got wrong."
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rubric_the_sweep_is_given_is_the_rubric_triage_applies() {
        // Two prompts applying two standards would produce re-grades that look
        // like a judgement call and are really a wording difference.
        let schema = crate::finding::finding_schema();
        let sweep =
            schema["properties"]["findings"]["items"]["properties"]["severity"]["description"]
                .as_str()
                .unwrap_or("");
        let triage_schema = triage_schema();
        let triage = triage_schema["properties"]["verdicts"]["items"]["properties"]["severity"]
            ["description"]
            .as_str()
            .unwrap_or("");
        assert_eq!(sweep, triage);
        assert_eq!(sweep, SEVERITY_RUBRIC);
    }

    #[test]
    fn a_verdict_reply_parses_into_verdicts() {
        let reply = json!({
            "verdicts": [{"id": "3", "severity": "low", "reason": "cosmetic"}]
        });
        let parsed: Result<SeverityVerdicts, _> = serde_json::from_value(reply);
        assert_eq!(parsed.map(|v| v.verdicts.len()).unwrap_or(0), 1);
    }
}
