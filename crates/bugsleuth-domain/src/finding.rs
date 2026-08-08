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
///
/// `findings` is deliberately **not** `#[serde(default)]`. It was, and a reply
/// that was valid JSON but simply had no `findings` key deserialized to an
/// empty list — reported as a lane that ran and found nothing, which is the one
/// outcome this crate exists to make impossible to fake. Kilo is the only
/// vendor with no schema enforcement and the one the code already documents as
/// producing malformed replies most often, so it is exactly where a `{}` reply
/// arrives. Required means such a reply is a schema error, which routes into
/// the repair attempt and, failing that, into an honest "not swept".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawFindings {
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
        // Made printable here, once, rather than at each place a report is
        // rendered. Every string below is attacker-influenced: the snippet is
        // code from the repository under review, and the model's own prose is
        // written after reading it. A terminal acts on an escape sequence
        // rather than showing it, so a repository could recolour, clear or
        // overwrite the report about itself — and this tool's output is a list
        // of security findings.
        //
        // At construction because there is more than one renderer, and fixing
        // the one that was found would have left the others. There is no path
        // to a `Finding` that does not come through here.
        Self {
            id,
            lane: lane.id(),
            model,
            title: crate::limits::printable(&raw.title),
            severity: raw.severity,
            anchor: VerifiedAnchor {
                file: crate::limits::printable(&anchor.file),
                snippet: crate::limits::printable(&anchor.snippet),
                ..anchor
            },
            explanation: crate::limits::printable(&raw.explanation),
            failure_scenario: crate::limits::printable(&raw.failure_scenario),
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
                            "description": crate::triage::SEVERITY_RUBRIC
                        },
                        "file": {
                            "type": "string",
                            "description": "Repository-relative path with forward slashes, e.g. src/lib.rs."
                        },
                        "line": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": u32::MAX,
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
#[path = "finding/tests.rs"]
mod tests;
