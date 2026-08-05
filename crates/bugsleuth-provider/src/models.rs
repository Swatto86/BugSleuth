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

mod codex_catalogue;
mod kilo_catalogue;

use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::ProviderError;
use crate::{codex, kilo, process};

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
        // Codex takes `-c model_reasoning_effort=<level>`, but which levels are
        // accepted belongs to the model, not the CLI: `gpt-5.6-sol` takes
        // `ultra` and `gpt-5.5` stops at `xhigh`. The vendor-wide list that
        // used to be here offered `max` on models that reject it and hid
        // `ultra` entirely — and it took precedence over the per-model answer,
        // so returning it would make the catalogue's detail dead data.
        "codex" => &[],
        // Kilo passes `--variant` straight through to whichever provider is
        // behind the model, so there is no vendor-wide set — most of its models
        // accept none at all, and some accept `instant`/`thinking` rather than a
        // ladder. Answered per model.
        _ => &[],
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
    // Nothing to fetch when there is nothing to check: an empty effort is the
    // CLI's own default, and only Codex is gated here — Claude's levels are
    // CLI-wide and checked by the planner, Kilo's variants are provider-specific.
    if effort.trim().is_empty() || vendor != "codex" {
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
        "codex" => Ok(codex_models().await),
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

/// Ask Codex for its catalogue, falling back to the known ids.
///
/// Never fails: a missing CLI, a timeout or an unparseable response all leave
/// the fallback list, because a menu that empties itself when a command fails
/// looks identical to a vendor with no models. What is lost in that case is the
/// per-model effort detail, which is why the fallback is the smaller claim.
async fn codex_models() -> VendorCatalogue {
    let Some(binary) = codex::binary_path() else {
        return fixed("Codex", CODEX_MODELS);
    };
    let output = process::run(process::Invocation {
        binary: &binary.to_string_lossy(),
        // Under `debug`, which is why this looked absent: neither `codex --help`
        // nor `codex models` mentions it.
        args: &["debug".to_string(), "models".to_string()],
        cwd: &std::env::temp_dir(),
        stdin: None,
        env: &[],
        // Reading a catalogue, not starting a model. Longer than this and
        // something is wrong; a dropdown must not hang open waiting.
        timeout: Duration::from_secs(60),
        what: "codex debug models",
    })
    .await;

    let Ok(output) = output else {
        return fixed("Codex", CODEX_MODELS);
    };
    let Some(entries) = codex_catalogue::parse(&output.stdout) else {
        return fixed("Codex", CODEX_MODELS);
    };

    VendorCatalogue {
        groups: vec![ModelGroup {
            label: "Codex".to_string(),
            models: entries.iter().map(|e| e.id.clone()).collect(),
        }],
        efforts_by_model: codex_catalogue::efforts(&entries),
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
    fn effort_not_supported_by_codex_model_is_rejected() {
        // Codex's vendor-wide list is empty on purpose: accepted levels are a
        // property of the model. `gpt-5.5` takes up to `xhigh` and rejects `max`.
        let mut catalogue = VendorCatalogue::default();
        catalogue.efforts_by_model.insert(
            "gpt-5.5".to_string(),
            vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
                "xhigh".to_string(),
            ],
        );

        assert!(effort_ok("codex", &catalogue, "gpt-5.5", "xhigh").is_ok());
        assert!(
            effort_ok("codex", &catalogue, "gpt-5.5", "max").is_err(),
            "`max` is not in the model's catalogue and must be refused"
        );
        // A model absent from the catalogue cannot be verified, so it is refused
        // rather than forwarded unchecked.
        assert!(effort_ok("codex", &catalogue, "gpt-absent", "high").is_err());
        // Empty effort is always fine — the CLI's own default.
        assert!(effort_ok("codex", &catalogue, "gpt-5.5", "").is_ok());
        assert!(effort_ok("codex", &catalogue, "gpt-5.5", "   ").is_ok());
        // Other vendors are not gated here; their efforts are checked elsewhere.
        assert!(effort_ok("claude", &catalogue, "opus", "max").is_ok());
    }

    #[test]
    fn only_claude_has_a_vendor_wide_effort_list() {
        // This test used to assert that Codex had one too, which was wrong.
        // `codex debug models` reports the accepted levels per model and they
        // differ: `gpt-5.6-sol` takes `ultra`, `gpt-5.5` stops at `xhigh`. A
        // vendor-wide list offered levels some models reject, hid `ultra`
        // entirely, and — because the window prefers the vendor-wide answer —
        // would have made the per-model catalogue dead data.
        assert!(!efforts("claude").is_empty());

        // Both of these answer per model instead, in `efforts_by_model`.
        assert!(efforts("codex").is_empty());
        assert!(efforts("kilo").is_empty());

        assert!(efforts("nonesuch").is_empty());
    }
}
