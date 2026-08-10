//! Fail fast when a selected provider cannot start a lane safely.

use bugsleuth_provider::process::redact_secrets;
use bugsleuth_provider::signin::SignIn;
use bugsleuth_provider::{claude, codex, kilo};

use super::Vendor;

pub(super) fn vendors_for(models: &[String]) -> Vec<Vendor> {
    [Vendor::Claude, Vendor::Codex, Vendor::Kilo]
        .into_iter()
        .filter(|vendor| models.iter().any(|model| Vendor::parse(model).0 == *vendor))
        .collect()
}

pub(super) fn kilo_permission_error() -> Option<String> {
    kilo::preflight::permission_gap().map(|gap| {
        format!(
            "kilo: not usable — Kilo sweeps require a deny-by-default `ask` agent because \
             reviewed source is attacker input: {gap}. Set `\"*\": \"deny\"`, then \
             explicitly allow only `read`, `glob`, and `grep` inside that agent's \
             permission block."
        )
    })
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

/// The distinct Kilo routes a plan will actually invoke.
///
/// Kilo authentication is per route, not per vendor, so reducing the plan to
/// "wants Kilo" and asking the configured default tested an invocation the run
/// was never going to make. Deduplicated because several lanes commonly share
/// one model, and each check costs a real minimal call.
fn kilo_routes(units: &[crate::plan::Unit]) -> Vec<(String, String)> {
    let mut routes: Vec<(String, String)> = Vec::new();
    for unit in units {
        let (vendor, model) = Vendor::parse(&unit.model);
        if vendor != Vendor::Kilo {
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
    let wants_kilo = vendors.contains(&Vendor::Kilo);
    let kilo_error = wants_kilo.then(kilo_permission_error).flatten();
    let check_kilo = wants_kilo && kilo_error.is_none();

    let (claude_result, codex_result, kilo_result) = tokio::join!(
        async {
            if wants_claude {
                Some((Vendor::Claude, claude::signin().await))
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
            if check_kilo {
                // Sequentially, and one per distinct selected route. Kilo
                // processes share a mutable credential store, so concurrent
                // invocations collide there; and each route authenticates
                // separately, so the configured default's answer says nothing
                // about the model this run will actually ask for.
                for (model, effort) in kilo_routes(units) {
                    results.push((Vendor::Kilo, kilo::signin_for(&model, &effort, None).await));
                }
            }
            results
        }
    );

    finish(
        [claude_result, codex_result]
            .into_iter()
            .flatten()
            .chain(kilo_result)
            .collect(),
        kilo_error.into_iter().collect(),
    )
}
