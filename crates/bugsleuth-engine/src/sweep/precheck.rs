//! Fail fast when a selected provider cannot start a lane safely.

use bugsleuth_provider::process::redact_secrets;
use bugsleuth_provider::signin::SignIn;
use bugsleuth_provider::{claude, codex, cursor, opencode};

use super::Vendor;

pub(super) fn vendors_for(models: &[String]) -> Vec<Vendor> {
    [
        Vendor::Claude,
        Vendor::Codex,
        Vendor::Cursor,
        Vendor::OpenCode,
    ]
    .into_iter()
    .filter(|vendor| models.iter().any(|model| Vendor::parse(model).0 == *vendor))
    .collect()
}

fn finish(checks: Vec<(Vendor, SignIn)>, extra_failures: Vec<String>) -> Result<(), String> {
    let mut failures: Vec<String> = checks
        .iter()
        .filter(|(_, result)| !result.usable())
        .map(|(vendor, result)| redact_secrets(&result.describe(vendor.label())))
        .collect();
    failures.extend(
        extra_failures
            .into_iter()
            .map(|error| redact_secrets(&error)),
    );

    if failures.is_empty() {
        return Ok(());
    }

    Err(format!(
        "Provider pre-check failed before any lane started:\n- {}\n\nFix those provider issues, then run the review again.",
        failures.join("\n- ")
    ))
}

#[cfg(test)]
pub(super) fn summarize(checks: Vec<(Vendor, SignIn)>) -> Result<(), String> {
    finish(checks, vec![])
}

/// The distinct routes a plan will actually invoke for one vendor.
///
/// Kilo, Kimi and Cursor authenticate per model rather than per vendor, so
/// reducing the plan to "wants this vendor" and asking the configured default
/// tests an invocation the run was never going to make. Deduplicated because
/// several lanes commonly share one model, and each check costs a real call.
fn routes_for(vendor_wanted: Vendor, units: &[crate::plan::Unit]) -> Vec<(String, String)> {
    let mut routes: Vec<(String, String)> = Vec::new();
    for unit in units {
        let (vendor, model) = Vendor::parse(&unit.model);
        if vendor != vendor_wanted {
            continue;
        }
        let route = (model.to_string(), unit.effort.trim().to_string());
        if !routes.contains(&route) {
            routes.push(route);
        }
    }
    routes
}

/// Check each selected provider once, concurrently, before lane work starts.
pub async fn selected(units: &[crate::plan::Unit]) -> Result<(), String> {
    let models: Vec<String> = units.iter().map(|unit| unit.model.clone()).collect();
    let models = &models[..];
    let vendors = vendors_for(models);
    let wants_claude = vendors.contains(&Vendor::Claude);
    let wants_codex = vendors.contains(&Vendor::Codex);
    let wants_cursor = vendors.contains(&Vendor::Cursor);
    let wants_opencode = vendors.contains(&Vendor::OpenCode);

    let (claude_result, codex_result, cursor_result, opencode_result) = tokio::join!(
        async {
            if wants_claude {
                Some((Vendor::Claude, claude::signin(None).await))
            } else {
                None
            }
        },
        async {
            if wants_codex {
                Some((Vendor::Codex, codex::signin().await))
            } else {
                None
            }
        },
        async {
            let mut results = Vec::new();
            if wants_cursor {
                for (model, _) in routes_for(Vendor::Cursor, units) {
                    results.push((Vendor::Cursor, cursor::signin_for(&model, None).await));
                }
            }
            results
        },
        async {
            let mut results = Vec::new();
            if wants_opencode {
                for (model, effort) in routes_for(Vendor::OpenCode, units) {
                    results.push((
                        Vendor::OpenCode,
                        opencode::signin_for(&model, &effort, None).await,
                    ));
                }
            }
            results
        }
    );

    finish(
        [claude_result, codex_result]
            .into_iter()
            .flatten()
            .chain(cursor_result)
            .chain(opencode_result)
            .collect(),
        vec![],
    )
}
