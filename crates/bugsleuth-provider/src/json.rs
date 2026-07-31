//! Reading a schema-shaped object out of a model's reply.
//!
//! Shared by every adapter. All of them ask their CLI to constrain output to a
//! JSON Schema, and all of them then have to cope with the reply not being
//! exactly what was asked for.

use serde_json::Value;

use crate::error::ProviderError;

/// Extract a schema-shaped value from the model's reply.
///
/// With `--json-schema` the reply should already be a JSON object. It is not
/// assumed to be: a schema-constrained reply can still arrive as a JSON *string*
/// containing the object, or wrapped in a code fence, and a run that dies here
/// wastes the whole invocation. Each fallback is cheap and strictly narrows what
/// counts as valid — anything that does not deserialize into the requested type
/// is still rejected.
pub(crate) fn structured<T: serde::de::DeserializeOwned>(
    result: &Value,
) -> Result<T, ProviderError> {
    let value = match result {
        Value::Object(_) => result.clone(),
        Value::String(text) => parse_embedded(text)?,
        Value::Null => return Err(ProviderError::Schema("the reply was empty".into())),
        other => {
            return Err(ProviderError::Schema(format!(
                "expected a structured object, got {}",
                kind(other)
            )));
        }
    };

    serde_json::from_value(value.clone()).map_err(|e| {
        ProviderError::Schema(format!(
            "{e}; reply began {:?}",
            head(&value.to_string(), 300)
        ))
    })
}

fn parse_embedded(text: &str) -> Result<Value, ProviderError> {
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
    Err(ProviderError::Schema(format!(
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

pub(crate) fn head(text: &str, max_chars: usize) -> String {
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
}
