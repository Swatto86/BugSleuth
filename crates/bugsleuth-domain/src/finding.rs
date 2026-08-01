//! What a model reports, and what survives verification.
//!
//! Two distinct types on purpose. `RawFinding` is untrusted: it is whatever a
//! model claimed, and its `file`/`line`/`snippet` may be invented. `Finding` can
//! only be constructed by the verify crate, from a raw finding whose snippet was
//! actually located in the file it names. Nothing that reaches a report can skip
//! that step, because the report type simply cannot hold an unverified anchor.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ids::{FindingId, LaneId, ModelId};
use crate::lane::Lane;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }
}

/// The envelope a lane sweep is asked to return. A single top-level object with
/// one array keeps the JSON Schema simple and gives the model an unambiguous
/// place to put "I found nothing" (an empty array) rather than prose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawFindings {
    #[serde(default)]
    pub findings: Vec<RawFinding>,
}

/// An unverified claim from a model. Every field here is suspect until checked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawFinding {
    /// One-line statement of the defect.
    pub title: String,
    pub severity: Severity,
    /// Repo-relative path, forward slashes.
    pub file: String,
    /// 1-indexed line the defect starts on.
    pub line: u32,
    /// Verbatim copy of the offending line(s) as they appear in the file. This
    /// is the field that makes the anchor mechanically checkable.
    pub snippet: String,
    /// Why it is wrong.
    pub explanation: String,
    /// Concrete inputs or state that trigger it, and the resulting bad outcome.
    pub failure_scenario: String,
    /// How to fix it, in enough detail for another model to do the work.
    ///
    /// The schema asks for this and the brief insists on it, but it is
    /// **optional to deserialize** on purpose. Only Claude has its output
    /// constrained by the schema; Codex and Kilo are merely asked. Making the
    /// field mandatory here meant one omission threw away the entire sweep —
    /// every finding in it, paid for and discarded, over a missing field the
    /// report is perfectly able to render as absent.
    #[serde(default)]
    pub fix: FixPlan,
}

/// Implementation instructions for a defect.
///
/// The point of this type is a specific reader: a *local* model, running on the
/// user's own machine, that will not have read the review, cannot ask a
/// question, and may not be strong enough to rediscover the reasoning. So every
/// field is written for someone starting from the file and nothing else.
///
/// Defaulted on deserialize so a sweep report written before fix plans existed
/// still loads. An empty plan is visibly empty in the report rather than
/// silently absent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FixPlan {
    /// The change to make, and why this is the right fix rather than a patch
    /// over the symptom.
    pub approach: String,
    /// Per-file instructions, precise enough to apply without further analysis.
    pub edits: Vec<FixEdit>,
    /// How to prove the fix worked — the test to add and the command to run.
    /// A fix nobody can check is a claim, not a fix.
    pub verification: String,
    /// What else this could break: other callers, callers' assumptions, data
    /// already written in the old shape.
    pub risks: String,
}

/// One file's worth of change.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FixEdit {
    /// Repo-relative path, forward slashes.
    pub file: String,
    /// Where in the file, named so it survives the line numbers moving —
    /// a function or symbol name rather than a bare offset.
    pub location: String,
    /// The change itself, concretely: what to replace with what.
    pub change: String,
}

/// A finding whose anchor was confirmed against the real file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: FindingId,
    pub lane: LaneId,
    pub model: ModelId,
    pub title: String,
    pub severity: Severity,
    pub anchor: VerifiedAnchor,
    pub explanation: String,
    pub failure_scenario: String,
    #[serde(default)]
    pub fix: FixPlan,
}

impl Finding {
    /// Assemble a verified finding. Deliberately takes an already-verified
    /// anchor: there is no path from a `RawFinding` to a `Finding` that does not
    /// go through the verify crate.
    pub fn new(
        id: FindingId,
        lane: Lane,
        model: ModelId,
        raw: RawFinding,
        anchor: VerifiedAnchor,
    ) -> Self {
        Self {
            id,
            lane: lane.id(),
            model,
            title: raw.title,
            severity: raw.severity,
            anchor,
            explanation: raw.explanation,
            failure_scenario: raw.failure_scenario,
            fix: raw.fix,
        }
    }
}

/// An anchor that was located in the file it claims to come from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedAnchor {
    /// Repo-relative path, as it exists on disk.
    pub file: String,
    /// The line the snippet was actually found on, which is authoritative.
    pub line: u32,
    /// The line the model claimed, kept so drift is visible in the report.
    pub claimed_line: u32,
    /// The snippet as it appears in the file (not as the model retyped it).
    pub snippet: String,
}

impl VerifiedAnchor {
    /// True when the model's line number was wrong and had to be corrected.
    pub fn was_corrected(&self) -> bool {
        self.line != self.claimed_line
    }
}

/// Schema for [`FixPlan`], kept separate because `json!` hits its recursion
/// limit when the whole thing is written as one literal.
///
/// The descriptions are the prompt. They name the reader explicitly — a smaller
/// local model that has not read the review — because a fix plan written for a
/// person who already understands the bug is not the same document.
fn fix_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "description": "Instructions for fixing this, written for a smaller model running locally that has NOT read your review and cannot ask you anything. Assume it can open the files and nothing else.",
        "required": ["approach", "edits", "verification", "risks"],
        "properties": {
            "approach": {
                "type": "string",
                "description": "What to change and why this is the right fix rather than a patch over the symptom. Name the root cause. If the same mistake appears in more than one place, say so here."
            },
            "edits": {
                "type": "array",
                "description": "One entry per file that must change, in the order they should be applied.",
                "items": fix_edit_schema()
            },
            "verification": {
                "type": "string",
                "description": "How to prove the fix worked: the test to add and where it goes, plus the exact command to run. Prefer a test that fails before the fix and passes after."
            },
            "risks": {
                "type": "string",
                "description": "What this could break: other callers of the code being changed, assumptions they make, and any data already stored in the old shape. Say 'none identified' only if you actually checked the callers."
            }
        }
    })
}

/// Schema for one [`FixEdit`].
fn fix_edit_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["file", "location", "change"],
        "properties": {
            "file": {
                "type": "string",
                "description": "Repository-relative path with forward slashes."
            },
            "location": {
                "type": "string",
                "description": "Where in the file, named so it still makes sense when line numbers move: the function, method, type or symbol to edit."
            },
            "change": {
                "type": "string",
                "description": "The edit itself, concretely enough to apply without further analysis: the code to replace and the code to replace it with. Include the replacement code, not a description of it."
            }
        }
    })
}

/// JSON Schema describing [`RawFindings`], for CLIs that can constrain output to
/// a schema (`claude --json-schema`). Hand-written rather than derived: adding a
/// schema-generation dependency to buy one 40-line literal is a poor trade, and
/// the wording of each `description` is prompt engineering that we want to edit
/// directly.
pub fn finding_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["findings"],
        "properties": {
            "findings": {
                "type": "array",
                "description": "Every defect found. Empty if none — never invent one to fill the array.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": [
                        "title", "severity", "file", "line",
                        "snippet", "explanation", "failure_scenario", "fix"
                    ],
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "One sentence stating the defect."
                        },
                        "severity": {
                            "type": "string",
                            "enum": ["critical", "high", "medium", "low"],
                            "description": "Judge by the consequence to a user, NOT by how interesting the bug is or how hard it was to find. critical: data loss, a security breach, or the application unusable. high: a common action fails or misleads the user, or an accessibility barrier locks someone out of an outcome entirely. medium: a real problem that has a workaround, or one that only affects an uncommon path. low: minor, cosmetic, or very rare. Ask what happens to the person using this, and whether they have another way to get what they wanted."
                        },
                        "file": {
                            "type": "string",
                            "description": "Repository-relative path with forward slashes, e.g. src/lib.rs."
                        },
                        "line": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "1-indexed line on which the offending code starts."
                        },
                        "snippet": {
                            "type": "string",
                            "description": "The offending line(s) copied EXACTLY as they appear in the file, character for character. This is checked against the file automatically and the finding is discarded if it does not match."
                        },
                        "explanation": {
                            "type": "string",
                            "description": "Why this code is wrong."
                        },
                        "failure_scenario": {
                            "type": "string",
                            "description": "Concrete inputs or state that trigger the defect, and what goes wrong as a result."
                        },
                        "fix": fix_schema()
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal value satisfying a schema object's `required` set.
    ///
    /// Recursive so a nested object is filled the same way its parent is, which
    /// is what lets the test keep checking schema-against-type as the shape
    /// grows rather than needing a new hand-written literal each time.
    fn required_shape(schema: &Value) -> Value {
        match schema["type"].as_str() {
            Some("array") => json!([required_shape(&schema["items"])]),
            Some("object") => {
                let mut object = serde_json::Map::new();
                for name in schema["required"].as_array().into_iter().flatten() {
                    let Some(name) = name.as_str() else { continue };
                    object.insert(
                        name.to_string(),
                        required_shape(&schema["properties"][name]),
                    );
                }
                Value::Object(object)
            }
            Some("integer") => json!(1),
            // The one string field with a closed set of values.
            _ if schema["enum"].is_array() => schema["enum"][0].clone(),
            _ => json!("x"),
        }
    }

    #[test]
    fn schema_and_type_agree_on_required_field_names() {
        let schema = finding_schema();
        let required = schema["properties"]["findings"]["items"]["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let names: Vec<&str> = required.iter().filter_map(Value::as_str).collect();

        // A finding shaped exactly like the schema's required set must deserialize.
        let mut object = serde_json::Map::new();
        for name in &names {
            let value = match *name {
                "line" => json!(1),
                "severity" => json!("high"),
                // Built from the schema's own nested required set rather than
                // hand-written, so this keeps checking the two agree instead of
                // freezing today's field names into the test.
                "fix" => required_shape(&fix_schema()),
                _ => json!("x"),
            };
            object.insert((*name).to_string(), value);
        }
        let parsed = serde_json::from_value::<RawFinding>(Value::Object(object));
        assert!(
            parsed.is_ok(),
            "schema required set does not build a RawFinding: {parsed:?}"
        );
    }

    #[test]
    fn empty_findings_array_parses_as_a_clean_sweep() {
        let parsed: RawFindings =
            serde_json::from_str(r#"{"findings":[]}"#).unwrap_or(RawFindings {
                findings: vec![RawFinding {
                    title: "sentinel".into(),
                    severity: Severity::Low,
                    file: String::new(),
                    line: 1,
                    snippet: String::new(),
                    explanation: String::new(),
                    failure_scenario: String::new(),
                    fix: Default::default(),
                }],
            });
        assert!(parsed.findings.is_empty());
    }

    #[test]
    fn a_finding_with_no_fix_plan_still_lands_rather_than_sinking_the_sweep() {
        // Only Claude's output is schema-constrained; the others are asked
        // nicely. A model that answers without `fix` must cost us that one
        // field, not the whole sweep it came in.
        let without = r#"{"findings":[{"title":"t","severity":"high","file":"a.rs",
            "line":2,"snippet":"x","explanation":"e","failure_scenario":"f"}]}"#;
        let parsed: RawFindings = serde_json::from_str(without)
            .unwrap_or_else(|e| panic!("a finding without a fix plan was rejected: {e}"));
        assert_eq!(parsed.findings.len(), 1);
        assert!(parsed.findings[0].fix.approach.is_empty());
        assert!(parsed.findings[0].fix.edits.is_empty());
    }

    #[test]
    fn severity_tells_the_model_what_the_levels_mean() {
        // It was a bare enum with no description, and models assigned it by
        // instinct: measured against independent assessment, 6 of 14 severities
        // were wrong, in both directions. Severity is the only thing the report
        // orders by, so an undefined scale made "worst first" unsupportable.
        let schema = finding_schema();
        let severity = &schema["properties"]["findings"]["items"]["properties"]["severity"];
        let text = severity["description"].as_str().unwrap_or_default();
        assert!(!text.is_empty(), "severity has no description at all");
        for level in ["critical", "high", "medium", "low"] {
            assert!(text.contains(level), "{level} is not defined");
        }
        assert!(
            text.contains("workaround"),
            "nothing distinguishes a blocking defect from an inconvenient one"
        );
    }
}
