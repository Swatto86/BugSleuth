//! Turning a model-to-lane configuration into a run.
//!
//! The unit of work is (model x lane). A model is configured once and then
//! assigned the lanes it covers — model is the primary entity, not lane —
//! because doubling models up on one lane is the point of the tool, not an
//! accident. Lanes buy coverage; two models on the same lane buy depth.
//!
//! The dangerous state this file exists to prevent: a lane with **no** model
//! assigned. A report that silently omits it reads exactly like a report where
//! that lane found nothing, and a reader would act on it. So every lane is
//! always enumerated, and one with no model assigned is carried through the
//! whole run as an explicit "not swept".

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use bugsleuth_domain::Lane;
use serde::Deserialize;

/// A configured model and the lanes it covers.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelPlan {
    /// `vendor:model`, or a bare model name for Claude.
    pub id: String,
    /// Lane slugs this model sweeps.
    pub lanes: Vec<String>,
    /// Reasoning effort for this model. Empty means the vendor's own default.
    #[serde(default)]
    pub effort: String,
    /// How many times to sweep each of this model's lanes.
    ///
    /// **Deliberate repetition, not a retry.** Three identical sweeps of the
    /// same fixture each returned five findings, but the union across them was
    /// six: the same model reliably finds slightly different things each time,
    /// and describes what it does find in different words. Asking twice is the
    /// cheapest recall there is — no second vendor, no second subscription.
    ///
    /// One by default. This costs a full sweep per pass and must be chosen.
    #[serde(default = "one")]
    pub passes: usize,
}

/// Serde needs a function; a bare `1` default is not expressible.
fn one() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub models: Vec<ModelPlan>,
}

/// One unit of work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub model: String,
    pub lane: Lane,
    /// Which pass this is, 1-based. Part of the unit so two passes of one model
    /// are two sweeps rather than one deduplicated away.
    pub pass: usize,
    /// Reasoning effort. Part of the unit because two efforts of the same model
    /// on the same lane are two different sweeps, not one run twice.
    pub effort: String,
}

/// A run, with the gaps made explicit.
#[derive(Debug)]
pub struct Plan {
    pub units: Vec<Unit>,
    /// Lanes no model was assigned to. Never silently dropped.
    pub uncovered: Vec<Lane>,
}

impl Plan {
    /// Units grouped so that no two for the same vendor run at once.
    ///
    /// Returned as a list of *batches*: everything in a batch is a different
    /// vendor and may run concurrently, and batches run one after another. This
    /// is the quota governor in its simplest useful form — with CLI subscriptions
    /// the binding constraint is rate limits rather than money, and hammering one
    /// vendor with parallel invocations is the fastest way to hit them.
    ///
    /// It is also what a real failure suggested: three CLIs started at once and
    /// one died with a silent non-zero exit, the shape these tools use for an
    /// overload.
    pub fn batches(&self) -> Vec<Vec<Unit>> {
        let mut remaining = self.units.clone();
        let mut batches = Vec::new();

        while !remaining.is_empty() {
            let mut batch: Vec<Unit> = Vec::new();
            let mut vendors: BTreeSet<String> = BTreeSet::new();
            remaining.retain(|unit| {
                let vendor = vendor_of(&unit.model);
                if vendors.contains(&vendor) {
                    return true;
                }
                vendors.insert(vendor);
                batch.push(unit.clone());
                false
            });
            batches.push(batch);
        }
        batches
    }
}

/// Reject an effort the vendor's CLI does not accept.
///
/// Checked here because here is free. An unrecognised effort is passed straight
/// to the CLI, which either rejects the invocation — costing a sweep of real
/// quota and tens of minutes to discover a typo — or ignores it, which is worse:
/// the run completes, the report looks normal, and nothing says the depth you
/// asked for was never applied.
///
/// Claude is checked here from its CLI-wide list. Codex is checked against its
/// per-model catalogue by the provider before invocation — its accepted levels
/// vary by model, so a synchronous vendor-wide list here would either reject
/// valid depths or wave through invalid ones. Kilo passes `--variant` through to
/// whichever provider is behind the model, so its accepted values are a property
/// of that model and are discovered at runtime; refusing what we cannot
/// enumerate would block valid configurations.
pub fn check_effort(id: &str, effort: &str) -> Result<()> {
    if effort.is_empty() {
        return Ok(());
    }
    let accepted = bugsleuth_provider::models::efforts(&vendor_of(id));
    if accepted.is_empty() || accepted.contains(&effort) {
        return Ok(());
    }
    anyhow::bail!(
        "model `{id}` asks for effort `{effort}`, which {} does not accept (try: {})",
        vendor_of(id),
        accepted.join(", ")
    )
}

/// The vendor prefix of a model spec. A bare name means Claude.
fn vendor_of(model: &str) -> String {
    match model.split_once(':') {
        Some((vendor, _)) if matches!(vendor, "claude" | "codex" | "kilo") => vendor.to_string(),
        _ => "claude".to_string(),
    }
}

/// Read a configuration file and enumerate the run.
pub fn load(path: &std::path::Path) -> Result<Plan> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let config: Config = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a valid BugSleuth config", path.display()))?;
    plan(&config)
}

pub fn plan(config: &Config) -> Result<Plan> {
    if config.models.is_empty() {
        anyhow::bail!("the configuration assigns no models, so there is nothing to run");
    }

    let mut units = Vec::new();
    for model in &config.models {
        if model.id.trim().is_empty() {
            anyhow::bail!("a configured model has an empty id");
        }
        check_effort(&model.id, model.effort.trim())?;
        // A config file is a file; the UI's picker is not the only way in.
        // Enumerating even a moderately large pass count hangs the run before
        // a single sweep is paid for, so refuse loudly (same class as
        // MAX_PROVE_TOP in src-tauri/src/outcome.rs).
        const MAX_PASSES: usize = 25;
        let passes = model.passes.max(1);
        if passes > MAX_PASSES {
            anyhow::bail!(
                "model `{}` asks for {passes} passes, more than the {MAX_PASSES} allowed; \
                 lower the `passes` value in the configuration",
                model.id
            );
        }
        for slug in &model.lanes {
            let lane: Lane = slug
                .parse()
                .with_context(|| format!("model `{}` is assigned an unknown lane", model.id))?;
            // A model listed twice against one lane is still a config slip,
            // and still collapses. Repetition is asked for with `passes`, where
            // it is visible and deliberate, rather than by writing the same
            // line out twice and hoping that means something.
            for pass in 1..=passes {
                let unit = Unit {
                    model: model.id.trim().to_string(),
                    lane,
                    effort: model.effort.trim().to_string(),
                    pass,
                };
                if !units.contains(&unit) {
                    units.push(unit);
                }
            }
        }
    }

    if units.is_empty() {
        anyhow::bail!("no model is assigned to any lane, so there is nothing to run");
    }

    let uncovered: Vec<Lane> = Lane::ALL
        .into_iter()
        .filter(|lane| !units.iter().any(|unit| unit.lane == *lane))
        .collect();

    Ok(Plan { units, uncovered })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod effort_gate_tests {
    /// Both commands must validate effort, and from the same function.
    ///
    /// The defect: `run` checked it and `sweep` did not, so a typo in --effort
    /// reached the vendor's CLI — either rejected after the sweep was paid for,
    /// or ignored, leaving a report that looks normal and never says the depth
    /// asked for was never applied. Two commands, one contract, one check.
    #[test]
    fn the_sweep_command_checks_effort_the_same_way_run_does() {
        let cli = include_str!("../../bugsleuth-cli/src/main.rs");
        let code: String = cli
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let sweep = code
            .split_once("async fn run_sweep")
            .expect("run_sweep is gone; this check needs rewriting")
            .1;
        let body = &sweep[..sweep.find("\nasync fn ").unwrap_or(sweep.len())];
        assert!(
            body.contains("check_effort"),
            "the sweep command no longer validates effort before spending a sweep"
        );
    }
}
