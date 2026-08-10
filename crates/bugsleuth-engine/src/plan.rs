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
    /// Ask supported CLIs to split this lane across parallel subagents.
    #[serde(default)]
    pub use_agents: bool,
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
    /// Whether this sweep asks the provider to delegate in parallel.
    pub use_agents: bool,
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
    /// Different vendors may run concurrently. Independent processes of one
    /// vendor stay serial because none publishes a safe process limit, and two
    /// real Kilo processes collided while updating its credential database.
    pub fn batches(&self) -> Vec<Vec<Unit>> {
        let mut remaining = self.units.clone();
        let mut batches = Vec::new();

        while !remaining.is_empty() {
            let mut batch: Vec<Unit> = Vec::new();
            let mut vendors = BTreeSet::new();
            remaining.retain(|unit| {
                let vendor = vendor_of(&unit.model);
                if vendors.insert(vendor) {
                    batch.push(unit.clone());
                    return false;
                }
                true
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
/// Claude's known aliases are checked here from their model-specific lists.
/// Codex is checked against its live per-model catalogue by the provider before
/// invocation. Kilo passes `--variant` through to whichever provider is behind
/// the model, so its accepted values are discovered at runtime; refusing what
/// we cannot enumerate would block valid configurations.
pub fn check_effort(id: &str, effort: &str) -> Result<()> {
    if effort.is_empty() {
        return Ok(());
    }
    let vendor = vendor_of(id);
    let model = canonical_spec(id);
    if let Some(accepted) = bugsleuth_provider::models::efforts_for(&vendor, &model) {
        if accepted.contains(&effort) {
            return Ok(());
        }
        let choices = if accepted.is_empty() {
            "leave effort at its default".to_string()
        } else {
            accepted.join(", ")
        };
        anyhow::bail!(
            "model `{id}` asks for effort `{effort}`, which {vendor} does not accept (try: {choices})"
        );
    }
    let accepted = bugsleuth_provider::models::efforts(&vendor);
    if accepted.is_empty() || accepted.contains(&effort) {
        return Ok(());
    }
    anyhow::bail!(
        "model `{id}` asks for effort `{effort}`, which {vendor} does not accept (try: {})",
        accepted.join(", ")
    )
}

/// The vendor prefix of a model spec. A bare name means Claude.
///
/// A prefix missing from this list answers "claude", silently — which is how a
/// `kimi:` model came to be checked against Claude's effort rules. Those accept
/// anything unlisted, so an effort Kimi has no flag for was waved through here
/// and then ignored by the CLI, with nothing saying the depth asked for was
/// never applied. Kept in step with [`crate::sweep::Vendor::parse`], which is
/// the enum this mirrors.
fn vendor_of(model: &str) -> String {
    match model.split_once(':') {
        Some((vendor, _)) if matches!(vendor, "claude" | "codex" | "kilo" | "kimi") => {
            vendor.to_string()
        }
        _ => "claude".to_string(),
    }
}

/// The canonical form of a model spec, so equivalent spellings are one unit.
///
/// `sonnet` and `claude:sonnet` invoke the same Claude model, but keying units
/// on the raw string scheduled and charged them as two — and their report
/// filenames differed, so resume could not find the earlier sweep. Both collapse
/// to the bare model here. `claude:` alone is kept, because it names Claude's
/// configured default rather than a specific model.
#[must_use]
pub fn canonical_spec(spec: &str) -> String {
    let spec = spec.trim();
    match spec.split_once(':') {
        Some(("claude", model)) if !model.trim().is_empty() => model.trim().to_string(),
        Some(("claude", _)) => "claude:".to_string(),
        Some((vendor, model)) if matches!(vendor, "codex" | "kilo" | "kimi") => {
            format!("{vendor}:{}", model.trim())
        }
        _ => spec.to_string(),
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
        // One canonical spelling before anything keys on it, so `sonnet` and
        // `claude:sonnet` are one scheduled, charged, resumable unit.
        let model_id = canonical_spec(&model.id);
        check_effort(&model_id, model.effort.trim())?;
        let vendor = vendor_of(&model_id);
        let claude_model = model_id.strip_prefix("claude:").unwrap_or(&model_id);
        if model.use_agents
            && vendor == "claude"
            && bugsleuth_provider::models::supports_ultracode(claude_model)
            && !model.effort.trim().is_empty()
        {
            anyhow::bail!(
                "model `{}` uses Claude Ultracode when agents are enabled; leave effort at its default",
                model.id
            );
        }
        // Asked of the same function the sweep asks, so a vendor added later
        // is refused here — before any quota is spent — rather than turning
        // every lane into NOT SWEPT after the run has been committed to.
        if model.use_agents
            && let Err(reason) =
                crate::sweep::agent_support(crate::sweep::Vendor::parse(&model_id).0, &model_id)
        {
            anyhow::bail!("model `{}` requests agents, but {reason}", model.id);
        }
        // A config file is a file; the UI's picker is not the only way in.
        // Enumerating even a moderately large pass count hangs the run before
        // a single sweep is paid for, so refuse loudly — the same class of
        // sender-supplied bound the desktop shell clamps for concurrency.
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
                    model: model_id.clone(),
                    lane,
                    effort: model.effort.trim().to_string(),
                    use_agents: model.use_agents,
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

// The scan that used to live here looked for the string `check_effort` inside
// `run_sweep`. `let _ = check_effort(...)` keeps the substring and throws the
// error away, so it could not fail for the defect it was written to catch.
// Replaced by crates/bugsleuth-cli/tests/sweep_effort.rs, which runs the real
// binary and asserts the sweep is refused before the provider is reached.
