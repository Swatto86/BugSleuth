//! What the model and effort dropdowns are filled from.
//!
//! Kept out of `commands.rs` because it is the one command that talks to a
//! vendor purely to *describe* what is available, rather than to review
//! anything — and because `commands.rs` is already at its size budget.

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
    /// Effort levels this vendor accepts, lowest first. Empty means it takes
    /// none, and the control must be disabled rather than silently ignored.
    pub efforts: Vec<String>,
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
        let (groups, error) = match models::available(vendor).await {
            Ok(groups) => (groups, None),
            Err(error) => (vec![], Some(error.to_string())),
        };
        out.push(VendorModels {
            vendor: vendor.to_string(),
            groups,
            efforts: models::efforts(vendor)
                .iter()
                .map(|e| (*e).to_string())
                .collect(),
            error,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_offered_vendor_has_effort_levels() {
        // A vendor in the dropdown with no efforts would render a dead control.
        // If one is ever added that genuinely has none, the UI must be taught to
        // disable it before this assertion is relaxed.
        for vendor in VENDORS {
            assert!(
                !models::efforts(vendor).is_empty(),
                "{vendor} offers no effort levels"
            );
        }
    }
}
