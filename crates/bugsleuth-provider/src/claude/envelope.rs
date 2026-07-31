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

pub(crate) fn parse(stdout: &str) -> Result<ResultEnvelope, ProviderError> {
    let envelope: ResultEnvelope =
        serde_json::from_str(stdout).map_err(|e| ProviderError::Envelope {
            vendor: VENDOR,
            detail: format!("{e}; output began {:?}", head(stdout, 200)),
        })?;

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
}
