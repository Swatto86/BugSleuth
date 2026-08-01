//! What you can actually pick, per vendor.
//!
//! Typing a model id by hand is how you discover, forty minutes into a sweep,
//! that you spelled it wrong — or worse, that you spelled a *real* model that
//! bills somewhere you did not intend. So the app offers a list.
//!
//! The three vendors differ in how much they will tell us, and this module is
//! honest about that rather than pretending to a uniformity that is not there:
//!
//! - **Kilo** publishes its whole catalogue (`kilo models`), and the first
//!   segment of every id is the billing route. That is the one list worth
//!   fetching live, because it is long, it changes, and getting the route wrong
//!   is the mistake that costs money.
//! - **Claude and Codex** have no list command. Their aliases are few, stable
//!   and documented in `--help`, so they are named here.
//!
//! Every list is a *suggestion*. A model id that is not on it must still be
//! usable, because a curated list goes stale and a tool that refuses a valid
//! model is worse than one that offers an incomplete menu.

use std::time::Duration;

use crate::error::ProviderError;
use crate::{kilo, process};

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

/// Effort levels a vendor accepts, in increasing order.
///
/// Empty means the vendor takes no effort setting, which the UI must show as
/// "not supported" rather than offering a control that silently does nothing.
#[must_use]
pub fn efforts(vendor: &str) -> &'static [&'static str] {
    match vendor {
        // `claude --effort <level>`.
        "claude" => &["low", "medium", "high", "xhigh", "max"],
        // `codex -c model_reasoning_effort=<level>`.
        "codex" => &["low", "medium", "high", "xhigh", "max"],
        // `kilo --variant <v>`: passed through to whichever provider is behind
        // the model, so these are the values that are common across them rather
        // than a set Kilo itself defines. An unsupported one is the provider's
        // to reject.
        "kilo" => &["low", "medium", "high"],
        _ => &[],
    }
}

/// Claude's documented aliases. Each always points at the newest of its family.
const CLAUDE_MODELS: &[&str] = &["fable", "opus", "sonnet", "haiku"];

/// Codex model ids. There is no list command, so this is what the CLI's own
/// help and defaults name.
const CODEX_MODELS: &[&str] = &["gpt-5.6-codex", "gpt-5.6-sol"];

/// Models to offer for a vendor.
///
/// Only Kilo costs anything to ask, and asking it starts no model — `kilo
/// models` reads a cached catalogue.
pub async fn available(vendor: &str) -> Result<Vec<ModelGroup>, ProviderError> {
    match vendor {
        "claude" => Ok(vec![named("Claude", CLAUDE_MODELS)]),
        "codex" => Ok(vec![named("Codex", CODEX_MODELS)]),
        "kilo" => kilo_models().await,
        _ => Err(ProviderError::NotFound {
            vendor: "unknown",
            hint: format!("no model list for vendor {vendor:?}"),
        }),
    }
}

fn named(label: &str, models: &[&str]) -> ModelGroup {
    ModelGroup {
        label: label.to_string(),
        models: models.iter().map(|m| (*m).to_string()).collect(),
    }
}

async fn kilo_models() -> Result<Vec<ModelGroup>, ProviderError> {
    let binary = kilo::binary_path().ok_or_else(|| ProviderError::NotFound {
        vendor: kilo::VENDOR,
        hint: "install the Kilo CLI to list its models".into(),
    })?;
    let output = process::run(process::Invocation {
        binary: &binary.to_string_lossy(),
        args: &["models".to_string()],
        cwd: &std::env::temp_dir(),
        stdin: None,
        env: &[],
        // Reading a cached catalogue. If it takes longer than this something is
        // wrong, and the app must not hang a dropdown open waiting for it.
        timeout: Duration::from_secs(60),
        what: "kilo models",
    })
    .await?;
    Ok(group_by_route(&output.stdout))
}

/// Split `provider/model` lines into groups, one per billing route.
///
/// Kept separate from the process call so it can be tested against real
/// captured output without running anything.
#[must_use]
pub fn group_by_route(listing: &str) -> Vec<ModelGroup> {
    // Insertion-ordered so the routes come out in the order defined below
    // rather than in whatever order the catalogue happens to list them.
    let mut groups: Vec<ModelGroup> = Vec::new();

    for line in listing.lines() {
        let id = line.trim();
        // The CLI prints a banner before the list; only `provider/model` lines
        // are models, and anything else is noise rather than an error.
        if id.is_empty() || !id.contains('/') || id.contains(' ') {
            continue;
        }
        let label = kilo::route_of(id).describe().to_string();
        match groups.iter_mut().find(|g| g.label == label) {
            Some(group) => group.models.push(id.to_string()),
            None => groups.push(ModelGroup {
                label,
                models: vec![id.to_string()],
            }),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kilo_models_are_grouped_by_who_pays_for_them() {
        // Abridged from real `kilo models` output. The grouping is the whole
        // point of the control: the same underlying model reached by two routes
        // bills to two different places, and the id alone does not say which.
        let listing = "\
kilo/anthropic/claude-opus-5
kilo/deepseek/deepseek-v4-pro
openrouter/anthropic/claude-opus-5
openrouter/z-ai/glm-4.6
openai/gpt-5.6-codex
ollama/qwen3-coder
";
        let groups = group_by_route(listing);
        let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels.len(), 4, "one group per route, got {labels:?}");

        let kilo_group = groups
            .iter()
            .find(|g| g.models.iter().any(|m| m.starts_with("kilo/")))
            .expect("no Kilo Gateway group");
        assert_eq!(kilo_group.models.len(), 2);

        // The same model under two routes must land in two different groups,
        // because that difference is exactly what the user is choosing between.
        let opus_groups: Vec<&str> = groups
            .iter()
            .filter(|g| g.models.iter().any(|m| m.ends_with("/claude-opus-5")))
            .map(|g| g.label.as_str())
            .collect();
        assert_eq!(opus_groups.len(), 2, "got {opus_groups:?}");
    }

    #[test]
    fn the_banner_and_blank_lines_are_not_mistaken_for_models() {
        // `kilo models` prints ASCII art first. Treating a banner line as a
        // model id would put unselectable rubbish in the dropdown.
        let listing = "\n██  ██ ████\n~~  ~~ ~~~~\n\nkilo/openai/gpt-latest\n";
        let groups = group_by_route(listing);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].models, vec!["kilo/openai/gpt-latest"]);
    }

    #[test]
    fn a_vendor_that_takes_no_effort_setting_says_so_with_an_empty_list() {
        // The UI keys off this to disable the control. A vendor missing from
        // the match must not silently get someone else's levels.
        assert!(efforts("nonesuch").is_empty());
        assert!(!efforts("claude").is_empty());
        assert!(!efforts("codex").is_empty());
        assert!(!efforts("kilo").is_empty());
    }
}
