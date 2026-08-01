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
        for slug in &model.lanes {
            let lane: Lane = slug
                .parse()
                .with_context(|| format!("model `{}` is assigned an unknown lane", model.id))?;
            let unit = Unit {
                model: model.id.trim().to_string(),
                lane,
                effort: model.effort.trim().to_string(),
            };
            // The same model assigned a lane twice is a config slip, not a
            // request to pay for the same sweep twice.
            if !units.contains(&unit) {
                units.push(unit);
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
mod tests {
    use super::*;

    fn config(entries: &[(&str, &[&str])]) -> Config {
        Config {
            models: entries
                .iter()
                .map(|(id, lanes)| ModelPlan {
                    id: (*id).to_string(),
                    lanes: lanes.iter().map(|l| (*l).to_string()).collect(),
                    effort: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_lane_with_no_model_is_reported_rather_than_quietly_omitted() {
        let plan = plan(&config(&[("sonnet", &["correctness"])]))
            .unwrap_or_else(|e| panic!("plan failed: {e}"));
        assert_eq!(plan.units.len(), 1);
        assert_eq!(plan.uncovered.len(), 3);
        assert!(plan.uncovered.contains(&Lane::Security));
    }

    #[test]
    fn a_fully_covered_run_reports_no_gaps() {
        let plan = plan(&config(&[(
            "sonnet",
            &["correctness", "security", "contract", "ux"],
        )]))
        .unwrap_or_else(|e| panic!("plan failed: {e}"));
        assert!(plan.uncovered.is_empty());
    }

    #[test]
    fn two_models_on_one_lane_is_two_units_because_that_is_the_point() {
        let plan = plan(&config(&[
            ("sonnet", &["correctness"]),
            ("codex:", &["correctness"]),
        ]))
        .unwrap_or_else(|e| panic!("plan failed: {e}"));
        assert_eq!(plan.units.len(), 2);
    }

    #[test]
    fn the_same_model_assigned_a_lane_twice_is_not_paid_for_twice() {
        let plan = plan(&config(&[("sonnet", &["correctness", "correctness"])]))
            .unwrap_or_else(|e| panic!("plan failed: {e}"));
        assert_eq!(plan.units.len(), 1);
    }

    #[test]
    fn a_configuration_that_would_run_nothing_is_an_error_not_an_empty_report() {
        assert!(plan(&config(&[])).is_err());
        assert!(plan(&config(&[("sonnet", &[])])).is_err());
    }

    #[test]
    fn an_unknown_lane_name_is_rejected_rather_than_silently_skipped() {
        assert!(plan(&config(&[("sonnet", &["correctnes"])])).is_err());
    }

    #[test]
    fn no_two_sweeps_of_the_same_vendor_share_a_batch() {
        let plan = plan(&config(&[
            ("sonnet", &["correctness", "security"]),
            ("claude:opus", &["contract"]),
            ("codex:", &["correctness"]),
            ("kilo:", &["ux"]),
        ]))
        .unwrap_or_else(|e| panic!("plan failed: {e}"));

        for batch in plan.batches() {
            let mut vendors: Vec<String> = batch.iter().map(|u| vendor_of(&u.model)).collect();
            let before = vendors.len();
            vendors.sort();
            vendors.dedup();
            assert_eq!(
                before,
                vendors.len(),
                "a batch ran one vendor twice at once"
            );
        }
    }

    #[test]
    fn batching_runs_every_unit_exactly_once() {
        let plan = plan(&config(&[
            ("sonnet", &["correctness", "security", "ux"]),
            ("codex:", &["correctness"]),
        ]))
        .unwrap_or_else(|e| panic!("plan failed: {e}"));

        let scheduled: usize = plan.batches().iter().map(Vec::len).sum();
        assert_eq!(scheduled, plan.units.len());
    }

    #[test]
    fn a_bare_model_name_is_treated_as_the_claude_vendor_for_batching() {
        assert_eq!(vendor_of("sonnet"), "claude");
        assert_eq!(vendor_of("claude:opus"), "claude");
        assert_eq!(vendor_of("codex:gpt"), "codex");
        assert_eq!(vendor_of("kilo:"), "kilo");
        // A model id that merely contains a colon is not a vendor prefix.
        assert_eq!(vendor_of("anthropic:claude-opus-5"), "claude");
    }
}
