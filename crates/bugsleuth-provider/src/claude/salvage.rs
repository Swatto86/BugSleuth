//! Recovering a sweep that stopped before it answered.
//!
//! `error_max_turns` is the most expensive failure this adapter has. The model
//! spent the whole budget doing the review — reading the repository, forming
//! findings — and then hit the ceiling before emitting the structured reply, so
//! everything it learned is discarded and the lane is reported as not swept.
//! Three sweeps died that way in one day, one of them a severity triage pass
//! that had no repository to read at all.
//!
//! The conversation is still there, though: the CLI's failure envelope carries
//! its session id. So the session is resumed once and asked only to answer,
//! with a tiny turn budget and no tools — everything expensive has already been
//! paid for, and the remaining work is transcription.
//!
//! Two rules, both the same as the Kilo repair path. It may not investigate
//! further: a resumed session with tools would simply exhaust a second budget.
//! And it may not invent — a model asked to summarise work it cannot fully
//! recall will fill the gaps, and a fabricated finding is what this project
//! exists to prevent. The anchor check still runs on whatever comes back, so an
//! invented snippet is discarded there regardless.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::{ResultEnvelope, Run, invoke_once};
use crate::error::ProviderError;

/// Enough to answer from what is already in the conversation, and no more. A
/// salvage that needs many turns is investigating, which is the thing this must
/// not do.
const SALVAGE_TURNS: u32 = 3;

pub(super) fn new_session_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        ^ (u128::from(std::process::id()) << 64)
        ^ u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
    value = (value & !(0xf000_u128 << 64)) | (0x4000_u128 << 64);
    value = (value & !(0xc000_u128 << 48)) | (0x8000_u128 << 48);
    let hex = format!("{value:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The session id of a run that failed *specifically* by exhausting its turns.
///
/// Returns nothing for any other failure. A sweep that died on a rate limit or
/// a signed-out CLI has no work in progress to recover, and resuming it would
/// spend quota to be told the same thing again.
pub(super) fn exhausted_session(stdout: &str) -> Option<String> {
    let envelope: ResultEnvelope = serde_json::from_str(stdout.trim()).ok()?;
    let subtype = envelope.subtype.as_deref()?;
    if !subtype.contains("max_turns") {
        return None;
    }
    envelope.session_id.filter(|id| !id.trim().is_empty())
}

const ASK: &str = "\
Your previous CLI run ended before you produced your answer. Everything you \
found is still in this conversation.

Output that answer now, as JSON matching the schema, and nothing else.

Do NOT read any more code, do NOT look anything up, and do NOT continue the \
review — there is no budget for it and the work is already done.

Report ONLY defects you actually established in this conversation. If you had \
not finished, report what you had, and return an empty list if that is nothing. \
An invented finding is far worse here than a short list: every snippet you \
report is checked against the file it names and a fabricated one is discarded, \
so inventing wastes the salvage and reports nothing either way.";

/// Recover work that is still inside a conversation, once. A successful CLI
/// envelope without a structured answer is the same incomplete outcome as a
/// timeout: starting the whole review over would only spend the quota twice.
pub(super) async fn recover<T: serde::de::DeserializeOwned>(
    first: Result<ResultEnvelope, ProviderError>,
    run: Run<'_>,
) -> Result<ResultEnvelope, ProviderError> {
    if run.resume.is_some() {
        return first;
    }
    let timeout = run.timeout.min(Duration::from_secs(300));
    match first {
        Ok(outcome) if !run.schema.is_null() => {
            let Err(original) = crate::json::structured::<T>(outcome.structured_result()) else {
                return Ok(outcome);
            };
            let Some(session) = outcome
                .session_id
                .as_deref()
                .or(run.session_id.as_deref())
                .filter(|session| !session.trim().is_empty())
            else {
                return Err(original);
            };
            recovered(
                original,
                salvage(
                    run.repo,
                    run.model,
                    session,
                    run.schema,
                    run.binary,
                    run.api_key,
                    timeout,
                )
                .await,
            )
        }
        Ok(outcome) => Ok(outcome),
        Err(
            original @ ProviderError::TurnsExhausted {
                session: Some(_), ..
            },
        ) => {
            let ProviderError::TurnsExhausted {
                session: Some(session),
                ..
            } = &original
            else {
                unreachable!()
            };
            let session = session.clone();
            recovered(
                original,
                salvage(
                    run.repo,
                    run.model,
                    &session,
                    run.schema,
                    run.binary,
                    run.api_key,
                    timeout,
                )
                .await,
            )
        }
        Err(original @ ProviderError::Process(crate::process::ProcessError::Timeout { .. }))
            if run.session_id.is_some() =>
        {
            recovered(
                original,
                salvage(
                    run.repo,
                    run.model,
                    run.session_id.as_deref().unwrap_or_default(),
                    run.schema,
                    run.binary,
                    run.api_key,
                    timeout,
                )
                .await,
            )
        }
        Err(other) => Err(other),
    }
}

fn recovered(
    original: ProviderError,
    recovery: Result<ResultEnvelope, ProviderError>,
) -> Result<ResultEnvelope, ProviderError> {
    let mut recovered = recovery.map_err(|recovery| ProviderError::Recovery {
        vendor: super::VENDOR,
        original: original.to_string(),
        recovery: recovery.to_string(),
    })?;
    recovered.salvaged = true;
    Ok(recovered)
}

/// Ask a turn-exhausted session for its answer. One attempt.
pub(super) async fn salvage(
    repo: &Path,
    model: &str,
    session: &str,
    schema: Value,
    binary: Option<&str>,
    api_key: Option<&str>,
    timeout: Duration,
) -> Result<ResultEnvelope, ProviderError> {
    // `invoke_once`, not `invoke`: a salvage is a single attempt by definition,
    // and going through the salvaging wrapper would be a cycle.
    invoke_once(Run {
        repo,
        model,
        effort: "",
        prompt: ASK,
        schema,
        // No tools. The review is over; this is transcription.
        available: "",
        allowed: "",
        denied: super::READ_ONLY_DENIED,
        max_turns: SALVAGE_TURNS,
        timeout,
        binary,
        api_key,
        session_id: None,
        resume: Some(session),
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_limit_failure_offers_its_session_for_salvage() {
        let stdout = r#"{"is_error":true,"subtype":"error_max_turns","session_id":"abc-123"}"#;
        assert_eq!(exhausted_session(stdout).as_deref(), Some("abc-123"));
    }

    #[test]
    fn any_other_failure_is_not_worth_resuming() {
        // A rate limit or a signed-out CLI has no work in progress to recover,
        // and resuming would spend quota to be told the same thing again.
        for other in [
            r#"{"is_error":true,"subtype":"error_during_execution","session_id":"abc"}"#,
            r#"{"is_error":true,"session_id":"abc"}"#,
            "not json at all",
            "",
        ] {
            assert_eq!(exhausted_session(other), None, "resumed for: {other}");
        }
    }

    #[test]
    fn a_turn_limit_with_no_session_id_cannot_be_salvaged() {
        let stdout = r#"{"is_error":true,"subtype":"error_max_turns","session_id":""}"#;
        assert_eq!(exhausted_session(stdout), None);
    }

    #[test]
    fn the_salvage_prompt_forbids_investigating_and_inventing() {
        // Both rules are load-bearing. Without the first it exhausts a second
        // budget; without the second it fills the gaps in what it recalls.
        assert!(ASK.contains("Do NOT read any more code"));
        assert!(ASK.contains("ONLY defects you actually established"));
        assert!(ASK.contains("empty list"));
    }
}
