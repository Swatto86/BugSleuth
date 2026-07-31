//! What you can actually pick, per vendor.
//!
//! Typing a model id by hand is how you discover, forty minutes into a sweep,
//! that you spelled it wrong — or worse, that you spelled a *real* model that
//! bills somewhere you did not intend. So the app offers a list.
//!
//! The three vendors differ in how much they will tell us, and this module is
//! honest about that rather than pretending to a uniformity that is not there:
//!
//! - **Kilo** publishes its whole catalogue, and it is the one list worth
//!   fetching live: it is long, it changes, and getting the billing route wrong
//!   is the mistake that costs money. It also says, per model, which reasoning
//!   efforts that model accepts — which is not uniform and is not always a
//!   graded scale. See `kilo_catalogue`.
//! - **Claude and Codex** have no list command. Their aliases are few, stable
//!   and documented in `--help`, so they are named here, and their effort
//!   levels belong to the CLI rather than to any one model.
//!
//! Every list is a *suggestion*. A model id that is not on it must still be
//! usable, because a curated list goes stale and a tool that refuses a valid
//! model is worse than one that offers an incomplete menu.

mod kilo_catalogue;

use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::ProviderError;
use crate::{kilo, process};

/// A named set of models shown together.
///
/// For Kilo the label is the billing route, which is the thing worth grouping
/// by: it is what decides whether a sweep spends your own key, your Kilo
/// subscription, or nothing at all.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ModelGroup {
    pub label: String,
    pub models: Vec<String>,
}

/// Everything the pickers for one vendor need.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct VendorCatalogue {
    pub groups: Vec<ModelGroup>,
    /// Efforts a *particular* model accepts, where the vendor says so.
    ///
    /// Empty for vendors whose effort levels belong to the CLI rather than the
    /// model. A model absent from this map has no per-model answer, which is
    /// not the same as accepting none — see [`efforts`].
    pub efforts_by_model: BTreeMap<String, Vec<String>>,
}

/// Effort levels a vendor accepts, for vendors where that is a property of the
/// CLI rather than of the model.
///
/// Empty means the answer is per-model instead, and the caller must look in
/// [`VendorCatalogue::efforts_by_model`]. Either way an empty result must show
/// as unavailable rather than as a control that silently does nothing.
#[must_use]
pub fn efforts(vendor: &str) -> &'static [&'static str] {
    match vendor {
        // `claude --effort <level>` — the CLI's own flag, same for every model.
        "claude" => &["low", "medium", "high", "xhigh", "max"],
        // `codex -c model_reasoning_effort=<level>`, likewise.
        "codex" => &["low", "medium", "high", "xhigh", "max"],
        // Kilo passes `--variant` straight through to whichever provider is
        // behind the model, so there is no vendor-wide set — most of its models
        // accept none at all, and some accept `instant`/`thinking` rather than a
        // ladder. Answered per model.
        _ => &[],
    }
}

/// Claude's documented aliases. Each always points at the newest of its family.
const CLAUDE_MODELS: &[&str] = &["fable", "opus", "sonnet", "haiku"];

/// Codex model ids. There is no list command, so this is what the CLI's own
/// help and defaults name.
const CODEX_MODELS: &[&str] = &["gpt-5.6-codex", "gpt-5.6-sol"];

/// Models to offer for a vendor, and what each one can be asked to do.
///
/// Only Kilo costs anything to ask, and asking it starts no model — `kilo
/// models` reads a cached catalogue.
pub async fn available(vendor: &str) -> Result<VendorCatalogue, ProviderError> {
    match vendor {
        "claude" => Ok(fixed("Claude", CLAUDE_MODELS)),
        "codex" => Ok(fixed("Codex", CODEX_MODELS)),
        "kilo" => kilo_models().await,
        _ => Err(ProviderError::NotFound {
            vendor: "unknown",
            hint: format!("no model list for vendor {vendor:?}"),
        }),
    }
}

fn fixed(label: &str, models: &[&str]) -> VendorCatalogue {
    VendorCatalogue {
        groups: vec![ModelGroup {
            label: label.to_string(),
            models: models.iter().map(|m| (*m).to_string()).collect(),
        }],
        efforts_by_model: BTreeMap::new(),
    }
}

async fn kilo_models() -> Result<VendorCatalogue, ProviderError> {
    let binary = kilo::binary_path().ok_or_else(|| ProviderError::NotFound {
        vendor: kilo::VENDOR,
        hint: "install the Kilo CLI to list its models".into(),
    })?;
    let output = process::run(process::Invocation {
        binary: &binary.to_string_lossy(),
        // `--verbose` rather than the bare list, because the bare list cannot
        // answer either question that matters: which account a model bills to,
        // and which efforts it accepts.
        args: &["models".to_string(), "--verbose".to_string()],
        cwd: &std::env::temp_dir(),
        stdin: None,
        env: &[],
        // Reading a cached catalogue. If it takes longer than this something is
        // wrong, and the app must not hang a dropdown open waiting for it.
        timeout: Duration::from_secs(60),
        what: "kilo models",
    })
    .await?;
    Ok(group_by_route(&output.stdout))
}

/// Turn a verbose listing into groups by billing route, plus per-model efforts.
///
/// Kept separate from the process call so it can be tested against real
/// captured output without running anything.
#[must_use]
pub fn group_by_route(listing: &str) -> VendorCatalogue {
    let mut catalogue = VendorCatalogue::default();

    for entry in kilo_catalogue::parse(listing) {
        // A `kilo/` id is Gateway unless the catalogue says the model bills to
        // a plan of your own. Nothing in the id itself distinguishes them.
        let route = match kilo::route_of(&entry.id) {
            kilo::Route::Gateway if entry.byok => kilo::Route::KiloByok,
            other => other,
        };
        let label = route.describe().to_string();
        match catalogue.groups.iter_mut().find(|g| g.label == label) {
            Some(group) => group.models.push(entry.id.clone()),
            None => catalogue.groups.push(ModelGroup {
                label,
                models: vec![entry.id.clone()],
            }),
        }
        if !entry.efforts.is_empty() {
            catalogue.efforts_by_model.insert(entry.id, entry.efforts);
        }
    }
    catalogue
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Abridged from real `kilo models --verbose` output, shape preserved.
    const VERBOSE: &str = r#"
kilo/ai21/jamba-large-1.7
{
  "id": "ai21/jamba-large-1.7",
  "variants": {},
  "hasUserByokAvailable": false
}
kilo/zai-coding/glm-5.2
{
  "id": "zai-coding/glm-5.2",
  "variants": { "high": {}, "max": {} },
  "hasUserByokAvailable": true
}
kilo/z-ai/glm-5.2
{
  "id": "z-ai/glm-5.2",
  "variants": { "low": {}, "medium": {}, "high": {} },
  "hasUserByokAvailable": false
}
openrouter/anthropic/claude-opus-5
{
  "id": "anthropic/claude-opus-5"
}
ollama/qwen3-coder
{
  "id": "qwen3-coder"
}
"#;

    #[test]
    fn a_kilo_model_billed_to_your_own_plan_is_not_filed_under_the_subscription() {
        // `kilo/z-ai/glm-5.2` spends Kilo Gateway credit and
        // `kilo/zai-coding/glm-5.2` spends a plan you bought from Z.ai. Same
        // model, same prefix, different bill — and the id cannot tell you.
        let catalogue = group_by_route(VERBOSE);
        let group_of = |id: &str| {
            catalogue
                .groups
                .iter()
                .find(|g| g.models.iter().any(|m| m == id))
                .unwrap_or_else(|| panic!("{id} is in no group"))
                .label
                .as_str()
        };
        assert!(group_of("kilo/zai-coding/glm-5.2").contains("BYOK"));
        assert!(group_of("kilo/z-ai/glm-5.2").contains("Gateway"));
        assert_ne!(
            group_of("kilo/zai-coding/glm-5.2"),
            group_of("kilo/z-ai/glm-5.2")
        );
    }

    #[test]
    fn efforts_are_recorded_per_model_and_only_where_the_model_has_any() {
        let catalogue = group_by_route(VERBOSE);
        assert_eq!(
            catalogue.efforts_by_model.get("kilo/zai-coding/glm-5.2"),
            Some(&vec!["high".to_string(), "max".to_string()])
        );
        assert_eq!(
            catalogue.efforts_by_model.get("kilo/z-ai/glm-5.2"),
            Some(&vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string()
            ]),
            "the same model on two routes can still accept different efforts"
        );
        // Absent rather than present-and-empty: the UI keys off absence to
        // disable the control, and an empty vec would read as "loading".
        assert!(
            !catalogue
                .efforts_by_model
                .contains_key("kilo/ai21/jamba-large-1.7")
        );
    }

    #[test]
    fn kilo_has_no_vendor_wide_effort_list_because_it_has_no_such_thing() {
        // Claude and Codex take an effort flag that means the same for every
        // model. Kilo forwards `--variant` to the provider, so the answer is
        // the model's, and claiming otherwise would offer levels that get
        // rejected.
        assert!(efforts("kilo").is_empty());
        assert!(!efforts("claude").is_empty());
        assert!(!efforts("codex").is_empty());
        assert!(efforts("nonesuch").is_empty());
    }
}
