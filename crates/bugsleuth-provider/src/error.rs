//! One error type for every adapter.
//!
//! The vendors differ in how they report trouble — exit codes, event streams,
//! stderr — but the engine above only needs to know which of a small number of
//! things went wrong, and whether retrying is worth it.

use crate::process::ProcessError;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("the {vendor} CLI could not be found. {hint}")]
    NotFound { vendor: &'static str, hint: String },
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("the {vendor} CLI exited with code {code}: {message}")]
    Failed {
        vendor: &'static str,
        code: i32,
        message: String,
    },
    #[error(
        "the {vendor} CLI exited with code {code} and produced no diagnostic output — usually a transient overload or rate limit"
    )]
    FailedSilently { vendor: &'static str, code: i32 },
    #[error("the {0} CLI produced no output")]
    Empty(&'static str),
    #[error("could not read the {vendor} CLI's response: {detail}")]
    Envelope {
        vendor: &'static str,
        detail: String,
    },
    #[error("the model's reply did not match the required structure: {0}")]
    Schema(String),
    #[error(
        "model `{model}` asks for effort `{effort}`, which {vendor} does not accept (try: {accepted})"
    )]
    InvalidEffort {
        vendor: &'static str,
        model: String,
        effort: String,
        accepted: String,
    },
    #[error("could not prepare the {vendor} CLI's working files: {detail}")]
    Scratch {
        vendor: &'static str,
        detail: String,
    },
    #[error("{vendor} {capability} is unavailable: {reason}")]
    CapabilityUnavailable {
        vendor: &'static str,
        capability: &'static str,
        reason: String,
    },
    /// The run used its whole turn budget before answering.
    ///
    /// Distinct from a plain failure because the review itself may already be
    /// done: everything the model found is still in the conversation this
    /// carries the id of, so it can be asked for the answer rather than paid
    /// for again from scratch.
    #[error(
        "the {vendor} CLI used its whole turn budget before answering — raise --max-turns, or narrow the scope"
    )]
    TurnsExhausted {
        vendor: &'static str,
        session: Option<String>,
    },
    #[error("{original}; automatic {vendor} session recovery also failed: {recovery}")]
    Recovery {
        vendor: &'static str,
        original: String,
        recovery: String,
    },
}

/// Whether a message reads as "this provider will not serve you right now".
///
/// A rate limit, a spent usage allowance, an overload. Deliberately one
/// definition, because two callers need it and they must agree: a single sweep
/// uses it to decide whether one more attempt is worth making, and a run uses
/// it to decide whether to stop rather than spend the rest of its units
/// discovering the same thing one refusal at a time.
///
/// Matched on the message text because that is what these CLIs give us — none
/// of them reports a machine-readable reason, and the text is also all that
/// survives into a stored report. Erring towards matching is the safe
/// direction here: the consequence of a false positive is one extra attempt, or
/// a run that stops early and says exactly how to continue it.
#[must_use]
pub fn looks_exhausted(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "rate limit",
        "rate-limit",
        "usage limit",
        "quota",
        "overloaded",
        "429",
        "503",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

impl ProviderError {
    /// Whether retrying might succeed. A silent non-zero exit is the shape these
    /// CLIs use for an overload or rate-limit blip, and a timeout may simply
    /// have been an unlucky moment — both are worth one more attempt. A schema
    /// mismatch or a missing binary will fail identically every time.
    pub fn is_transient(&self) -> bool {
        match self {
            ProviderError::FailedSilently { .. } => true,
            // A clean exit with no answer is the shape of a provider-side empty
            // completion — a blip, not a permanent condition. Worth one more
            // attempt before a whole lane is reported as never run.
            ProviderError::Empty(_) => true,
            ProviderError::Process(ProcessError::Timeout { .. }) => true,
            ProviderError::Failed { message, .. } => {
                let lower = message.to_lowercase();
                looks_exhausted(message)
                    || ["timeout", "try again"].iter().any(|n| lower.contains(n))
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {

    /// One definition of "this provider will not serve you right now", because
    /// two very different decisions rest on it: whether a single sweep is worth
    /// one more attempt, and whether a whole run should stop rather than
    /// rediscover the same refusal once per remaining unit.
    #[test]
    fn a_spent_allowance_is_recognised_however_the_cli_words_it() {
        for message in [
            "429 Too Many Requests",
            "You have exceeded your rate limit",
            "rate-limited; retry after 60s",
            "usage limit reached for this month",
            "Quota exceeded for this organization",
            "Error 503: the service is overloaded",
        ] {
            assert!(looks_exhausted(message), "not recognised: {message}");
        }

        // These are failures, but not the provider refusing to serve — treating
        // them as a spent allowance would abandon a run over a bad prompt or a
        // missing binary, which no amount of waiting fixes.
        for message in [
            "Model not found: claude:nope",
            "the response did not match the required schema",
            "ENOENT: no such file or directory",
            "permission denied",
        ] {
            assert!(!looks_exhausted(message), "wrongly recognised: {message}");
        }
    }
    use super::*;

    #[test]
    fn a_silent_failure_is_worth_retrying_but_a_missing_binary_is_not() {
        assert!(
            ProviderError::FailedSilently {
                vendor: "claude",
                code: 1
            }
            .is_transient()
        );
        assert!(
            ProviderError::Empty("kilo").is_transient(),
            "a silent empty completion is the shape of a transient blip"
        );
        assert!(
            !ProviderError::NotFound {
                vendor: "codex",
                hint: String::new()
            }
            .is_transient()
        );
        assert!(!ProviderError::Schema("bad".into()).is_transient());
    }

    #[test]
    fn a_rate_limit_message_is_recognised_however_it_is_worded() {
        for message in ["Rate limit exceeded", "server overloaded", "HTTP 429"] {
            assert!(
                ProviderError::Failed {
                    vendor: "claude",
                    code: 1,
                    message: message.to_string(),
                }
                .is_transient(),
                "{message} should be transient"
            );
        }
        assert!(
            !ProviderError::Failed {
                vendor: "claude",
                code: 1,
                message: "unknown model".to_string(),
            }
            .is_transient()
        );
    }
}
