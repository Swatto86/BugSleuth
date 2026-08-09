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

/// Check each selected provider once, concurrently, before lane work starts.
pub async fn selected(models: &[String]) -> Result<(), String> {
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
            if check_kilo {
                Some((Vendor::Kilo, kilo::signin().await))
            } else {
                None
            }
        }
    );

    finish(
        [claude_result, codex_result, kilo_result]
            .into_iter()
            .flatten()
            .collect(),
        kilo_error.into_iter().collect(),
    )
}
