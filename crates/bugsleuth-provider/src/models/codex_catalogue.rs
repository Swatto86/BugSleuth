//! Reading `codex debug models`.
//!
//! This tool shipped with two Codex models hardcoded, taken from the CLI's help
//! text and its default, because `codex --help` and `codex models` both suggest
//! there is no listing. There is: it is under `debug`, and it returns the whole
//! catalogue as JSON.
//!
//! Two things in it cannot be guessed from a model id:
//!
//! - **`visibility`** — the catalogue includes entries the CLI does not offer,
//!   such as `codex-auto-review`. Listing one would put a model in the menu
//!   that is not meant to be chosen directly.
//! - **`supported_reasoning_levels`** — the efforts *that model* accepts. They
//!   are not uniform. `gpt-5.6-sol` accepts `ultra`; `gpt-5.5` and everything
//!   below it stop at `xhigh`. A single vendor-wide list, which is what this
//!   replaced, offers levels some models reject and hides `ultra` entirely.
//!
//! The response is large — a few hundred kilobytes, most of it each model's
//! system prompt — so it is read with a cap and everything unrecognised is
//! ignored.

use std::collections::BTreeMap;

use serde::Deserialize;

/// One model as the catalogue describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) id: String,
    /// Reasoning efforts this model accepts, weakest first as the CLI orders
    /// them. Empty means it takes none, which the window must show as
    /// unavailable rather than guess at.
    pub(super) efforts: Vec<String>,
}

#[derive(Deserialize)]
struct Catalogue {
    #[serde(default)]
    models: Vec<Model>,
}

/// Only the fields that are used. Everything else — display names, pricing
/// tiers, the base instructions — is ignored so the catalogue can grow without
/// this needing to know.
#[derive(Deserialize)]
struct Model {
    slug: String,
    /// `"list"` for a model the CLI offers. Anything else is not for choosing.
    #[serde(default)]
    visibility: String,
    #[serde(default)]
    supported_reasoning_levels: Vec<Level>,
}

#[derive(Deserialize)]
struct Level {
    effort: String,
}

/// Parse the catalogue. `None` when the output is not the JSON we expect, so
/// the caller can fall back rather than present an empty menu as the answer.
pub(super) fn parse(output: &str) -> Option<Vec<Entry>> {
    // The CLI may print progress before the object. Start at the first brace
    // rather than requiring the whole stream to be JSON.
    let start = output.find('{')?;
    let catalogue: Catalogue = serde_json::from_str(output[start..].trim()).ok()?;

    let entries: Vec<Entry> = catalogue
        .models
        .into_iter()
        .filter(|m| m.visibility == "list")
        .filter(|m| !m.slug.trim().is_empty())
        .map(|m| Entry {
            id: m.slug,
            efforts: m
                .supported_reasoning_levels
                .into_iter()
                .map(|l| l.effort)
                .collect(),
        })
        .collect();

    // An empty list is not a catalogue. Returning one would replace a working
    // menu with nothing and look like a vendor with no models.
    (!entries.is_empty()).then_some(entries)
}

/// Efforts per model, for the map the window reads.
pub(super) fn efforts(entries: &[Entry]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|e| (e.id.clone(), e.efforts.clone()))
        .collect()
}

#[cfg(test)]
#[path = "codex_catalogue/tests.rs"]
mod tests;
