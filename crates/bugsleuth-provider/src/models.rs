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
//!   and documented in `--help`, so they are named here. Their effort levels
//!   vary by model and are recorded alongside those known models.
//!
//! Every list is a *suggestion*. A model id that is not on it must still be
//! usable, because a curated list goes stale and a tool that refuses a valid
//! model is worse than one that offers an incomplete menu.

mod codex_catalogue;
mod efforts;
mod kilo_catalogue;

use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::ProviderError;
use crate::{codex, cursor, kilo, kimi, process};

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
/// Only Kilo costs anything to ask, and asking it starts no model — `kilo
/// models` reads a cached catalogue.
pub async fn available(vendor: &str) -> Result<VendorCatalogue, ProviderError> {
    match vendor {
        "claude" => Ok(claude_models()),
        "codex" => Ok(codex_models().await),
        "kilo" => kilo_models().await,
        "kimi" => Ok(kimi_models()),
        "cursor" => cursor_models().await,
        _ => Err(ProviderError::NotFound {
            vendor: "unknown",
            hint: format!("no model list for vendor {vendor:?}"),
        }),
    }
}

async fn cursor_models() -> Result<VendorCatalogue, ProviderError> {
    let ids = cursor::list_model_ids().await;
    if ids.is_empty() {
        // CLI missing, timed out, or unparseable — keep a typed-in default.
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

/// Kimi's menu, read from the CLI's own configuration.
///
/// There is no `models` list command, and which aliases exist depends on the
/// account — a subscription and a bring-your-own-key setup reach different
/// sets. So the menu comes from the same file the CLI reads rather than from a
/// list written here, which would offer models this user may not have.
fn kimi_models() -> VendorCatalogue {
    kimi::catalogue()
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

async fn kilo_models() -> Result<VendorCatalogue, ProviderError> {
    let binary = kilo::binary_path().ok_or_else(|| ProviderError::NotFound {
        vendor: kilo::VENDOR,
        hint: "install the Kilo CLI to list its models".into(),
    })?;
    // Fail fast rather than queue: this fills a dropdown, and a menu that hangs
    // for the length of a sweep is worse than one that says why it is empty.
    // Kilo processes share a mutable credential store, so this must not run
    // beside a live sweep or apply.
    let Some(_operation) = kilo::try_operation_guard() else {
        return Err(ProviderError::NotFound {
            vendor: kilo::VENDOR,
            hint: "a Kilo review or apply is running; its model list cannot be read at the \
                   same time"
                .into(),
        });
    };
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
    kilo_catalogue_from_output(output)
}

/// Interpret a finished `kilo models` invocation.
///
/// `process::run` returns `Ok` for a process that launched and exited non-zero,
/// so a bare `group_by_route` on the stdout of a failed run parses nothing into
/// an empty catalogue and returns it as success — Kilo then looks like a vendor
/// with no models rather than one whose listing failed. Both a non-zero exit and
/// an empty successful parse are errors here.
fn kilo_catalogue_from_output(
    output: process::CliOutput,
) -> Result<VendorCatalogue, ProviderError> {
    if !output.succeeded() {
        let code = output.code.unwrap_or(-1);
        let diagnostic = if output.stderr.trim().is_empty() {
            process::preview(output.stdout.trim(), 2_000)
        } else {
            process::preview(output.stderr.trim(), 2_000)
        };
        return Err(if diagnostic.is_empty() {
            ProviderError::FailedSilently {
                vendor: kilo::VENDOR,
                code,
            }
        } else {
            ProviderError::Failed {
                vendor: kilo::VENDOR,
                code,
                message: diagnostic,
            }
        });
    }

    let catalogue = group_by_route(&output.stdout);
    if catalogue.groups.is_empty() {
        return Err(ProviderError::Envelope {
            vendor: kilo::VENDOR,
            detail: "the model listing contained no model IDs".to_string(),
        });
    }
    Ok(catalogue)
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
#[path = "models/tests.rs"]
mod tests;
