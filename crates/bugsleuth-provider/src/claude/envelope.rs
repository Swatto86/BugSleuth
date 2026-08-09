//! Unwrapping the CLI's response.
//!
//! Two layers have to come apart. The outer layer is the CLI's own transcript
//! envelope from `--output-format json`, which reports success, cost and session
//! id. The inner layer is the model's actual reply, which is where our findings
//! live. `--output-format json` says nothing about the shape of the inner reply,
//! so it still has to be parsed and validated against our own schema.

use serde::Deserialize;

use super::{ProviderError, ResultEnvelope};
use crate::json::head;

use super::VENDOR;

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

impl Usage {
    pub fn to_text(&self) -> String {
        format!(
            "input_tokens={}, output_tokens={}, cache_creation_input_tokens={}, cache_read_input_tokens={}",
            self.input_tokens,
            self.output_tokens,
            self.cache_creation_input_tokens,
            self.cache_read_input_tokens
        )
    }
}

impl ResultEnvelope {
    pub(crate) fn structured_result(&self) -> &serde_json::Value {
        self.structured_output.as_ref().unwrap_or(&self.result)
    }
}

/// Whatever the CLI said about a failure, from its own envelope on stdout.
///
/// Used only when the process exited non-zero and said nothing on stderr. The
/// envelope may well be truncated in that case, so a parse failure falls back
/// to showing the start of the output raw — something a person can act on beats
/// "produced no diagnostic output", which is what this replaced.
pub(crate) fn failure_text(stdout: &str) -> String {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return String::new();
    }
    let Ok(envelope) = serde_json::from_str::<ResultEnvelope>(stdout) else {
        return head(stdout, 2000);
    };
    let detail = envelope
        .result
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| envelope.subtype.clone().unwrap_or_default());
    if detail.is_empty() {
        head(stdout, 2000)
    } else {
        head(&detail, 2000)
    }
}

pub(crate) fn parse(stdout: &str) -> Result<ResultEnvelope, ProviderError> {
    let envelope: ResultEnvelope =
        serde_json::from_str(stdout).map_err(|e| ProviderError::Envelope {
            vendor: VENDOR,
            detail: format!("{e}; output began {:?}", head(stdout, 200)),
        })?;

    // A turn-budget exhaustion is recoverable and must be reported as such
    // whatever the exit code was. This used to be detected only on the
    // non-zero-exit path, so the identical failure delivered with a clean exit
    // fell through to a generic error and the whole sweep was thrown away —
    // exactly the loss the salvage exists to prevent.
    if envelope.is_error
        && envelope
            .subtype
            .as_deref()
            .is_some_and(|s| s.contains("max_turns"))
    {
        return Err(ProviderError::TurnsExhausted {
            vendor: VENDOR,
            session: envelope.session_id.filter(|id| !id.trim().is_empty()),
        });
    }

    if envelope.is_error {
        let detail = envelope
            .result
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| envelope.subtype.clone().unwrap_or_default());
        return Err(ProviderError::Failed {
            vendor: VENDOR,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_envelope_becomes_an_error_not_an_empty_sweep() {
        let stdout =
            r#"{"is_error":true,"subtype":"error_max_turns","result":"hit the turn limit"}"#;
        assert!(parse(stdout).is_err());
    }

    #[test]
    fn a_normal_envelope_is_read_without_complaint() {
        let stdout = r#"{"is_error":false,"result":{"findings":[]},"num_turns":3}"#;
        let parsed = parse(stdout);
        assert_eq!(parsed.map(|e| e.num_turns).unwrap_or(None), Some(3));
    }

    #[test]
    fn a_schema_reply_uses_structured_output_instead_of_the_text_result() {
        let stdout = r#"{"is_error":false,"result":"Review complete.","structured_output":{"findings":[]},"num_turns":3}"#;
        let parsed = parse(stdout).expect("valid envelope");
        assert_eq!(
            parsed.structured_result(),
            &serde_json::json!({"findings": []})
        );
    }

    #[test]
    fn a_legacy_schema_reply_can_still_fall_back_to_result() {
        let stdout = r#"{"is_error":false,"result":{"findings":[]},"num_turns":3}"#;
        let parsed = parse(stdout).expect("valid legacy envelope");
        assert_eq!(
            parsed.structured_result(),
            &serde_json::json!({"findings": []})
        );
    }

    #[test]
    fn a_failure_is_explained_from_the_envelope_when_stderr_says_nothing() {
        // The case this exists for: a non-zero exit whose only account of itself
        // is on stdout. Two real sweeps were reported as having said nothing.
        let stdout =
            r#"{"is_error":true,"subtype":"error_max_turns","result":"hit the turn limit"}"#;
        assert_eq!(failure_text(stdout), "hit the turn limit");
    }

    #[test]
    fn an_envelope_with_no_result_falls_back_to_its_subtype() {
        let stdout = r#"{"is_error":true,"subtype":"error_during_execution"}"#;
        assert_eq!(failure_text(stdout), "error_during_execution");
    }

    #[test]
    fn output_that_is_not_an_envelope_is_still_shown_rather_than_dropped() {
        // A killed CLI truncates mid-write. Raw output a person can read beats
        // reporting that there was none.
        assert!(failure_text("panicked at src/main.rs: oh no").contains("oh no"));
        assert!(failure_text(r#"{"is_error":true,"#).contains("is_error"));
        assert!(failure_text("   ").is_empty());
    }

    #[test]
    fn a_turn_exhaustion_is_recoverable_whatever_the_exit_code_was() {
        // Detected only on the non-zero-exit path for a while, so the identical
        // failure delivered with a clean exit fell through to a generic error
        // and the sweep was discarded rather than salvaged.
        let stdout =
            r#"{"is_error":true,"subtype":"error_max_turns","session_id":"s-1","result":""}"#;
        match parse(stdout) {
            Err(ProviderError::TurnsExhausted { session, .. }) => {
                assert_eq!(session.as_deref(), Some("s-1"));
            }
            other => panic!("expected a recoverable exhaustion, got {other:?}"),
        }
    }

    #[test]
    fn any_other_envelope_error_is_still_an_ordinary_failure() {
        let stdout = r#"{"is_error":true,"subtype":"error_during_execution","result":"boom"}"#;
        assert!(matches!(parse(stdout), Err(ProviderError::Failed { .. })));
    }

    #[test]
    fn usage_token_counts_are_surfaced_as_text() {
        let usage = Usage {
            input_tokens: 50_000,
            output_tokens: 5_000,
            cache_creation_input_tokens: 1_000,
            cache_read_input_tokens: 2_000,
        };
        let text = usage.to_text();
        assert!(text.contains("input_tokens=50000"), "{text}");
        assert!(text.contains("output_tokens=5000"), "{text}");
        assert!(text.contains("cache_creation_input_tokens=1000"), "{text}");
        assert!(text.contains("cache_read_input_tokens=2000"), "{text}");
    }
}
