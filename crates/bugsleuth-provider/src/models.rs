//! What you can actually pick, per vendor.
//!
//! Typing a model id by hand is how you discover, forty minutes into a sweep,
//! that you spelled it wrong — or worse, that you spelled a *real* model that
//! bills somewhere you did not intend. So the app offers a list.
//!
//! Claude offers documented aliases. Codex, Cursor and OpenCode expose model
//! catalogues; OpenCode includes globally configured local provider routes.
//!
//! Every list is a *suggestion*. A model id that is not on it must still be
//! usable, because a curated list goes stale and a tool that refuses a valid
//! model is worse than one that offers an incomplete menu.

mod codex_catalogue;
mod efforts;
mod opencode_catalogue;
mod verbose_catalogue;

use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::ProviderError;
use crate::{claude, codex, cursor, opencode, process};

/// Vendors the desktop and CLI know about, in menu order.
pub const VENDORS: &[&str] = &["claude", "codex", "cursor", "opencode"];

/// Whether this vendor's CLI is on the machine, without starting it.
///
/// Cheaper than `--version` and cheaper than a catalogue fetch: PATH / known
/// install locations only. Menus and sign-in checks consult this first so a
/// missing CLI is never offered or probed as if it were usable.
#[must_use]
pub fn cli_installed(vendor: &str) -> bool {
    match vendor {
        "claude" => claude::binary_path().is_some(),
        "codex" => codex::binary_path().is_some(),
        "cursor" => cursor::binary_path().is_some(),
        "opencode" => opencode::binary_path().is_some(),
        _ => false,
    }
}

/// A named set of models shown together.
///
/// OpenCode groups by provider route.
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
    /// A model absent from this map has no per-model answer, which is not the
    /// same as accepting none — see [`efforts`].
    pub efforts_by_model: BTreeMap<String, Vec<String>>,
}

/// Effort levels a vendor accepts, for vendors where that is a property of the
/// CLI rather than of the model.
///
/// Empty means the answer is per-model instead, and the caller must look in
/// [`VendorCatalogue::efforts_by_model`]. Either way an empty result must show
/// as unavailable rather than as a control that silently does nothing.
#[must_use]
pub fn efforts(_vendor: &str) -> &'static [&'static str] {
    &[]
}

#[cfg(test)]
pub(crate) use efforts::effort_ok;
pub use efforts::efforts_for;
pub(crate) use efforts::validate_effort;

/// Claude's documented aliases. Each always points at the newest of its family.
const CLAUDE_MODELS: &[&str] = &["fable", "opus", "sonnet", "haiku"];

/// Whether Claude Code documents Ultracode for this model selection.
#[must_use]
pub fn supports_ultracode(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    matches!(model.as_str(), "" | "fable" | "opus" | "sonnet")
        || [
            "claude-fable-5",
            "claude-opus-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-sonnet-5",
        ]
        .iter()
        .any(|family| model.contains(family))
}

fn claude_models() -> VendorCatalogue {
    let mut catalogue = fixed("Claude", CLAUDE_MODELS);
    for model in CLAUDE_MODELS {
        if let Some(levels) = efforts_for("claude", model) {
            catalogue.efforts_by_model.insert(
                (*model).to_string(),
                levels.iter().map(|level| (*level).to_string()).collect(),
            );
        }
    }
    catalogue
}

/// Codex model ids. There is no list command, so this is what the CLI's own
/// help and defaults name.
const CODEX_MODELS: &[&str] = &["gpt-5.6-codex", "gpt-5.6-sol"];

/// Models to offer for a vendor, and what each one can be asked to do.
///
/// Refuses when the CLI is not installed — a fixed list for a missing binary
/// is how Claude and Codex used to appear usable on machines that cannot run
/// them. Catalogue discovery does not start a model.
pub async fn available(vendor: &str) -> Result<VendorCatalogue, ProviderError> {
    if !cli_installed(vendor) {
        return Err(not_installed(vendor));
    }
    match vendor {
        "claude" => Ok(claude_models()),
        "codex" => Ok(codex_models().await),
        "cursor" => cursor_models().await,
        "opencode" => opencode_catalogue::available().await,
        _ => Err(ProviderError::NotFound {
            vendor: "unknown",
            hint: format!("no model list for vendor {vendor:?}"),
        }),
    }
}

fn not_installed(vendor: &str) -> ProviderError {
    match vendor {
        "opencode" => ProviderError::NotFound { vendor: "opencode", hint: "Install OpenCode and configure a cloud or local provider.".into() },
        "claude" => ProviderError::NotFound {
            vendor: "claude",
            hint: "Install it with `npm install -g @anthropic-ai/claude-code` and sign in by running `claude` once.".into(),
        },
        "codex" => ProviderError::NotFound {
            vendor: "codex",
            hint: "Install the Codex CLI and sign in with `codex login`.".into(),
        },
        "cursor" => ProviderError::NotFound {
            vendor: "cursor",
            hint: "Install the Cursor Agent CLI (`agent`) and run `agent login`.".into(),
        },
        _ => ProviderError::NotFound {
            vendor: "unknown",
            hint: format!("no model list for vendor {vendor:?}"),
        },
    }
}

async fn cursor_models() -> Result<VendorCatalogue, ProviderError> {
    let ids = cursor::list_model_ids().await;
    if ids.is_empty() {
        // Binary is present (caller checked) but the list was empty, timed out,
        // or unparseable — keep a typed-in default rather than blanking the box.
        return Ok(fixed("Cursor", &["auto"]));
    }
    Ok(VendorCatalogue {
        groups: vec![ModelGroup {
            label: "Cursor".to_string(),
            models: ids,
        }],
        efforts_by_model: BTreeMap::new(),
    })
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
/// Never fails once the CLI is present: a timeout or an unparseable response
/// leaves the fallback list, because a menu that empties itself when a command
/// fails looks identical to a vendor with no models. What is lost in that case
/// is the per-model effort detail, which is why the fallback is the smaller claim.
/// Missing CLI is refused by [`available`] before this runs.
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
    codex_catalogue_from_output(output)
}

fn codex_catalogue_from_output(output: process::CliOutput) -> VendorCatalogue {
    if !output.succeeded() {
        return fixed("Codex", CODEX_MODELS);
    }
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

#[cfg(test)]
#[path = "models/tests.rs"]
mod tests;
