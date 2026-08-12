//! Which reasoning efforts a model accepts.
//!
//! Split from `models.rs` at the hard line cap, along a real seam: that file
//! answers "which models exist", and this one answers "what depth may one be
//! asked for". The two change for different reasons — a catalogue goes stale
//! when a vendor ships a model, these rules change when a vendor ships a flag.
//!
//! Three shapes of answer, and the difference matters. `Some(levels)` is a
//! known list. `Some(&[])` is a model that accepts **none**, which is a real
//! answer and is refused. `None` is *unknown*, and is waved through — a curated
//! list goes stale, and refusing a valid model is worse than forwarding one.

use super::{VendorCatalogue, available, supports_ultracode};
use crate::error::ProviderError;

const CLAUDE_OPUS_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CLAUDE_FABLE_EFFORTS: &[&str] = CLAUDE_OPUS_EFFORTS;
const CLAUDE_SONNET_EFFORTS: &[&str] = CLAUDE_OPUS_EFFORTS;
const NO_EFFORTS: &[&str] = &[];

#[must_use]
pub fn efforts_for(vendor: &str, model: &str) -> Option<&'static [&'static str]> {
    match (vendor, model.trim()) {
        ("claude", "fable") => Some(CLAUDE_FABLE_EFFORTS),
        ("claude", "opus") => Some(CLAUDE_OPUS_EFFORTS),
        ("claude", "sonnet") => Some(CLAUDE_SONNET_EFFORTS),
        ("claude", "haiku") => Some(NO_EFFORTS),
        // Kimi has no reasoning-depth flag at all. `Some(NO_EFFORTS)` rather
        // than `None`: `None` means "unknown, allow anything", which would let
        // an effort through to a CLI that has nowhere to put it.
        ("kimi", _) => Some(NO_EFFORTS),
        // Effort is encoded in Cursor model ids (e.g. `...-high`), not a separate flag.
        ("cursor", _) => Some(NO_EFFORTS),
        _ => None,
    }
}

/// Validate a Codex effort against its selected model's catalogue entry.
///
/// The contract [`efforts`] documents: an empty vendor-wide list means the
/// answer is per-model, in [`VendorCatalogue::efforts_by_model`]. Codex returns
/// an empty vendor-wide list for exactly that reason, so an effort forwarded to
/// `model_reasoning_effort` must be checked against the model's own accepted
/// levels rather than waved through. Fetches the catalogue and defers to
/// [`effort_ok`], which is pure so it can be tested without the CLI.
pub(crate) async fn validate_effort(
    vendor: &'static str,
    model: &str,
    effort: &str,
) -> Result<(), ProviderError> {
    let effort = effort.trim();
    if effort.is_empty() {
        return Ok(());
    }
    if vendor == "claude" {
        if effort == "ultracode" && supports_ultracode(model) {
            return Ok(());
        }
        let Some(accepted) = efforts_for(vendor, model) else {
            return Ok(());
        };
        if accepted.contains(&effort) {
            return Ok(());
        }
        return Err(ProviderError::InvalidEffort {
            vendor,
            model: model.trim().to_string(),
            effort: effort.to_string(),
            accepted: if accepted.is_empty() {
                "leave effort at its default".to_string()
            } else {
                accepted.join(", ")
            },
        });
    }
    if vendor != "codex" {
        return Ok(());
    }
    let catalogue = available(vendor).await?;
    effort_ok(vendor, &catalogue, model, effort)
}

/// Whether a Codex effort is one the model's catalogue entry accepts.
///
/// Pure, so the rule can be tested without invoking a CLI. A model with no entry
/// cannot be verified and is refused rather than forwarded unchecked — an empty
/// vendor-wide list means "consult the per-model catalogue", never "accept
/// anything".
pub(crate) fn effort_ok(
    vendor: &'static str,
    catalogue: &VendorCatalogue,
    model: &str,
    effort: &str,
) -> Result<(), ProviderError> {
    let effort = effort.trim();
    if effort.is_empty() || vendor != "codex" {
        return Ok(());
    }
    let model = model.trim();
    let accepted =
        catalogue
            .efforts_by_model
            .get(model)
            .ok_or_else(|| ProviderError::InvalidEffort {
                vendor,
                model: model.to_string(),
                effort: effort.to_string(),
                accepted: "choose a listed model or leave effort at its default".to_string(),
            })?;
    if accepted.iter().any(|level| level == effort) {
        return Ok(());
    }
    Err(ProviderError::InvalidEffort {
        vendor,
        model: model.to_string(),
        effort: effort.to_string(),
        accepted: accepted.join(", "),
    })
}
