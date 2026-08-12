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
        Value::String(text) => return parse_embedded(text),
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

fn parse_embedded<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, ProviderError> {
    const MAX_EMBEDDED_CHARS: usize = 256 * 1024;
    const MAX_BRACE_STARTS: usize = 128;

    let trimmed = strip_fence(text.trim());
    if trimmed.len() > MAX_EMBEDDED_CHARS {
        return Err(ProviderError::Schema(format!(
            "the reply JSON candidate was too large ({} bytes)",
            trimmed.len()
        )));
    }
    let exact_error = match serde_json::from_str::<T>(trimmed) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    // Bound the recovery fallback over brace-heavy malformed output. Without
    // this, a malicious reply can drive repeated partial deserialization
    // attempts and consume excessive CPU.
    let brace_starts = trimmed.chars().filter(|&c| c == '{').count();
    if brace_starts > MAX_BRACE_STARTS {
        return Err(ProviderError::Schema(format!(
            "the reply contained too many JSON object start candidates ({brace_starts}); refusing embedded recovery"
        )));
    }
    // Last resort: try each object start directly as the requested type. The
    // deserializer deliberately does not require EOF, so trailing prose or a
    // later object cannot make a complete typed answer disappear.
    for (start, _) in trimmed
        .char_indices()
        .filter(|(_, ch)| *ch == '{')
        .take(MAX_BRACE_STARTS)
    {
        let mut deserializer = serde_json::Deserializer::from_str(&trimmed[start..]);
        if let Ok(value) = T::deserialize(&mut deserializer) {
            return Ok(value);
        }
    }
    // Preserve the useful field-level diagnostic for a complete JSON value
    // that simply has the wrong shape. Parsing as Value here affects only the
    // error text; an arbitrary object is never accepted as the answer.
    if serde_json::from_str::<Value>(trimmed).is_ok() {
        return Err(ProviderError::Schema(format!(
            "{exact_error}; reply began {:?}",
            head(trimmed, 300)
        )));
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
    fn braces_in_leading_prose_do_not_hide_the_json_object() {
        let value = Value::String(["Expected shape {findings:[...]\n", ONE].concat());
        let parsed = structured::<RawFindings>(&value);
        assert_eq!(parsed.map(|f| f.findings.len()).unwrap_or(0), 1);
    }

    #[test]
    fn an_unrelated_json_object_does_not_win_over_the_typed_answer() {
        let value = Value::String([r#"Metadata: {"note":"example"}. Final: "#, ONE].concat());
        let parsed = structured::<RawFindings>(&value);
        assert_eq!(parsed.map(|f| f.findings.len()).unwrap_or(0), 1);
    }

    #[test]
    fn candidate_offsets_follow_utf8_boundaries() {
        let value = Value::String(["Résumé 🔎 {not JSON; answer: ", ONE].concat());
        let parsed = structured::<RawFindings>(&value);
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
    fn braces_in_malformed_output_are_bounded() {
        let prefix = "{".repeat(10_000);
        let value = Value::String(format!("{prefix}{ONE}"));
        let err =
            structured::<RawFindings>(&value).expect_err("should reject brace-heavy recovery");
        assert!(
            err.to_string()
                .contains("too many JSON object start candidates"),
            "unexpected error: {err}"
        );
    }
}
