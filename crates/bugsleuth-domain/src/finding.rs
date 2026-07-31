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
                        "snippet", "explanation", "failure_scenario"
                    ],
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "One sentence stating the defect."
                        },
                        "severity": {
                            "type": "string",
                            "enum": ["critical", "high", "medium", "low"]
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
                }],
            });
        assert!(parsed.findings.is_empty());
    }
}
