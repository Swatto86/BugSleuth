//! Unwrapping the CLI's response.
//!
//! Two layers have to come apart. The outer layer is the CLI's own transcript
//! envelope from `--output-format json`, which reports success, cost and session
//! id. The inner layer is the model's actual reply, which is where our findings
//! live. `--output-format json` says nothing about the shape of the inner reply,
//! so it still has to be parsed and validated against our own schema.

use serde::Deserialize;
use serde_json::Value;

use super::{ClaudeError, ResultEnvelope};

/// Token accounting for one invocation. Cost is the *equivalent* API price; the
/// call itself is covered by the subscription.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

pub(crate) fn parse(stdout: &str) -> Result<ResultEnvelope, ClaudeError> {
    let envelope: ResultEnvelope = serde_json::from_str(stdout)
        .map_err(|e| ClaudeError::Envelope(format!("{e}; output began {:?}", head(stdout, 200))))?;

    if envelope.is_error {
        let detail = envelope
            .result
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| envelope.subtype.clone().unwrap_or_default());
        return Err(ClaudeError::Failed {
            code: 0,
            message: if detail.is_empty() {
                "the CLI reported an error with no detail".to_string()
            } else {
                head(&detail, 2000)
            },
        });
    }
    Ok(envelope)
}

/// Extract a schema-shaped value from the model's reply.
///
/// With `--json-schema` the reply should already be a JSON object. It is not
/// assumed to be: a schema-constrained reply can still arrive as a JSON *string*
/// containing the object, or wrapped in a code fence, and a run that dies here
/// wastes the whole invocation. Each fallback is cheap and strictly narrows what
/// counts as valid — anything that does not deserialize into the requested type
/// is still rejected.
pub(crate) fn structured<T: serde::de::DeserializeOwned>(result: &Value) -> Result<T, ClaudeError> {
    let value = match result {
        Value::Object(_) => result.clone(),
        Value::String(text) => parse_embedded(text)?,
        Value::Null => return Err(ClaudeError::Schema("the reply was empty".into())),
        other => {
            return Err(ClaudeError::Schema(format!(
                "expected a structured object, got {}",
                kind(other)
            )));
        }
    };

    serde_json::from_value(value.clone()).map_err(|e| {
        ClaudeError::Schema(format!(
            "{e}; reply began {:?}",
            head(&value.to_string(), 300)
        ))
    })
}

fn parse_embedded(text: &str) -> Result<Value, ClaudeError> {
    let trimmed = strip_fence(text.trim());
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }
    // Last resort: the model prefixed or suffixed prose around the object.
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    if let (Some(start), Some(end)) = (start, end)
        && end > start
        && let Ok(value) = serde_json::from_str::<Value>(&trimmed[start..=end])
    {
        return Ok(value);
    }
    Err(ClaudeError::Schema(format!(
        "the reply contained no JSON object; it began {:?}",
        head(trimmed, 300)
    )))
}

/// Strip a leading code fence, and only a leading one. Searching the whole
/// string for a fence would mistake a triple-backtick inside a quoted snippet
/// for the opening delimiter and slice away the real JSON.
fn strip_fence(text: &str) -> &str {
    for open in ["```json", "```JSON", "```", "~~~json", "~~~"] {
        if let Some(rest) = text.strip_prefix(open) {
            let close = if open.starts_with('~') { "~~~" } else { "```" };
            return match rest.rfind(close) {
                Some(end) => rest[..end].trim(),
                None => rest.trim(),
            };
        }
    }
    text
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn head(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::RawFindings;
    use serde_json::json;

    const ONE: &str = r#"{"findings":[{"title":"t","severity":"high","file":"a.rs","line":2,"snippet":"x","explanation":"e","failure_scenario":"f"}]}"#;

    #[test]
    fn reads_findings_delivered_as_a_json_object() {
        let value: Value = serde_json::from_str(ONE).unwrap_or(Value::Null);
        let parsed = structured::<RawFindings>(&value);
        assert_eq!(parsed.map(|f| f.findings.len()).unwrap_or(0), 1);
    }

    #[test]
    fn reads_findings_delivered_as_a_json_string() {
        let value = Value::String(ONE.to_string());
        let parsed = structured::<RawFindings>(&value);
        assert_eq!(parsed.map(|f| f.findings.len()).unwrap_or(0), 1);
    }

    #[test]
    fn reads_findings_wrapped_in_a_code_fence() {
        let value = Value::String(format!("```json\n{ONE}\n```"));
        let parsed = structured::<RawFindings>(&value);
        assert_eq!(parsed.map(|f| f.findings.len()).unwrap_or(0), 1);
    }

    #[test]
    fn a_backtick_run_inside_a_snippet_does_not_truncate_the_json() {
        let with_ticks = json!({
            "findings": [{
                "title": "t",
                "severity": "low",
                "file": "a.md",
                "line": 1,
                "snippet": "```rust",
                "explanation": "e",
                "failure_scenario": "f"
            }]
        })
        .to_string();
        let parsed = structured::<RawFindings>(&Value::String(with_ticks));
        assert_eq!(parsed.map(|f| f.findings.len()).unwrap_or(0), 1);
    }

    #[test]
    fn a_reply_that_is_not_findings_is_rejected_rather_than_silently_empty() {
        let value = Value::String("I could not find any issues.".into());
        assert!(structured::<RawFindings>(&value).is_err());
    }

    #[test]
    fn a_reply_missing_required_fields_is_rejected() {
        let value: Value =
            serde_json::from_str(r#"{"findings":[{"title":"t"}]}"#).unwrap_or(Value::Null);
        assert!(structured::<RawFindings>(&value).is_err());
    }

    #[test]
    fn an_error_envelope_becomes_an_error_not_an_empty_sweep() {
        let stdout =
            r#"{"is_error":true,"subtype":"error_max_turns","result":"hit the turn limit"}"#;
        assert!(parse(stdout).is_err());
    }
}
