//! What the model and effort dropdowns are filled from.
//!
//! Kept out of `commands.rs` because it is the one command that talks to a
//! vendor purely to *describe* what is available, rather than to review
//! anything — and because `commands.rs` is already at its size budget.

use std::collections::BTreeMap;

use bugsleuth_engine::models::{self, ModelGroup};
use serde::Serialize;

/// Vendors offered in the model dropdown, in the order they appear.
const VENDORS: [&str; 4] = ["claude", "codex", "kilo", "kimi"];

/// One vendor's menu.
#[derive(Serialize)]
pub struct VendorModels {
    pub vendor: String,
    /// Models to offer, grouped. For Kilo the group label is the billing route,
    /// which is the difference the user is actually choosing between.
    pub groups: Vec<ModelGroup>,
    /// Effort levels the *CLI* accepts, weakest first, for vendors where that
    /// is a property of the CLI rather than of the model. Empty means the
    /// answer is per model instead — see `efforts_by_model`.
    pub efforts: Vec<String>,
    /// Effort levels a particular model accepts. Kilo forwards `--variant` to
    /// whichever provider is behind the model, so this is the only truthful
    /// answer for it: most of its models take none, and some take
    /// `instant`/`thinking` rather than a ladder.
    pub efforts_by_model: BTreeMap<String, Vec<String>>,
    /// Why this vendor's list is empty, when it is. Carried per vendor so one
    /// missing CLI does not blank the other two.
    pub error: Option<String>,
}

/// Every vendor's models and effort levels.
///
/// Never fails as a whole: a vendor that cannot be asked comes back with an
/// empty list and the reason, because a dropdown that silently offers nothing
/// looks identical to a vendor with no models.
#[tauri::command]
pub async fn available_models() -> Vec<VendorModels> {
    let mut out = Vec::with_capacity(VENDORS.len());
    for vendor in VENDORS {
        let (catalogue, error) = match models::available(vendor).await {
            // An empty list with nothing said is the failure this module's note
            // warns about: it looks identical to a vendor with no models. Kimi
            // has no list command, so the box is a free-text field and this is
            // the only place that can say what to put in it.
            Ok(catalogue) if catalogue.groups.is_empty() => {
                (catalogue, Some(empty_list_reason(vendor)))
            }
            Ok(catalogue) => (catalogue, None),
            Err(error) => (models::VendorCatalogue::default(), Some(error.to_string())),
        };
        out.push(VendorModels {
            vendor: vendor.to_string(),
            groups: catalogue.groups,
            efforts: models::efforts(vendor)
                .iter()
                .map(|e| (*e).to_string())
                .collect(),
            efforts_by_model: catalogue.efforts_by_model,
            error,
        });
    }
    out
}

/// Why a vendor offers nothing, in words that say what to do about it.
fn empty_list_reason(vendor: &str) -> String {
    match vendor {
        // Only shown when the list really is empty, which for Kimi means no
        // readable ~/.kimi-code/config.toml — the menu is read from that file.
        "kimi" => concat!(
            "No models found in ~/.kimi-code/config.toml. Install the Kimi Code CLI and ",
            "run `kimi` then /login, or type an alias by hand — the box accepts one."
        )
        .to_string(),
        other => format!("{other} returned no models"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vendor_answers_about_effort_either_for_the_cli_or_for_each_model() {
        // Every current vendor answers per model. Claude supports effort only
        // on particular families, Kilo forwards `--variant` to the selected
        // model, and Codex reports the accepted levels per model too.
        //
        // This test asserted Codex was CLI-wide until the catalogue was read
        // and said otherwise. It is listed here rather than inferred, so adding
        // a vendor forces a deliberate answer instead of defaulting to one.
        const PER_MODEL: [&str; 3] = ["claude", "kilo", "codex"];

        // And a third answer, which is not the same as having no answer: Kimi
        // has no reasoning-depth flag of any kind, so the only truthful thing
        // its control can do is refuse. `efforts_for` says so explicitly rather
        // than leaving it unknown, and that is what makes the refusal happen —
        // an unknown vendor is waved through on the assumption it knows best.
        const NO_EFFORT: [&str; 1] = ["kimi"];

        // What must never happen is a vendor with none of the three, which
        // would render a control that silently does nothing.
        for vendor in VENDORS {
            let cli_wide = !models::efforts(vendor).is_empty();
            let per_model = PER_MODEL.contains(&vendor);
            let refuses = NO_EFFORT.contains(&vendor);
            assert_eq!(
                u8::from(cli_wide) + u8::from(per_model) + u8::from(refuses),
                1,
                "{vendor} must answer about effort exactly one way, not several or none"
            );
        }

        // The refusal is real, not merely declared in the list above.
        for vendor in NO_EFFORT {
            assert_eq!(
                models::efforts_for(vendor, "anything"),
                Some(&[][..]),
                "{vendor} is listed as accepting no effort but does not say so"
            );
        }
    }
}
