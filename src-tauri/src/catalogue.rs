//! What the model and effort dropdowns are filled from.
//!
//! Kept out of `commands.rs` because it is the one command that talks to a
//! vendor purely to *describe* what is available, rather than to review
//! anything — and because `commands.rs` is already at its size budget.

use std::collections::BTreeMap;

use bugsleuth_engine::models::{self, ModelGroup};
use serde::Serialize;

/// Vendors offered in the model dropdown, in the order they appear.
const VENDORS: [&str; 3] = ["claude", "codex", "kilo"];

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

        // What must never happen is a vendor with neither, which would render a
        // control that does nothing.
        for vendor in VENDORS {
            let cli_wide = !models::efforts(vendor).is_empty();
            let per_model = PER_MODEL.contains(&vendor);
            assert!(
                cli_wide != per_model,
                "{vendor} must answer about effort exactly one way, not both or neither"
            );
        }
    }
}
