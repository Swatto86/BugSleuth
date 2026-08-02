//! Turning a finished run into what the window is handed.
//!
//! The gap lines, the fix prompt and the proof pass: everything that happens
//! *after* the sweeps. Kept apart from the commands that start them because it
//! is a different job, and because the two together crossed the line cap.

use std::time::Duration;

use bugsleuth_engine::orchestrate;
use tauri::Emitter;

use crate::settings::Settings;

/// Every lane nobody reviewed, as one line each.
pub(crate) fn gap_lines(report: &orchestrate::RunReport) -> Vec<String> {
    report
        .gaps
        .iter()
        .map(|gap| {
            format!(
                "{} lane, by {} — {}",
                gap.lane,
                gap.model.as_deref().unwrap_or("nobody"),
                gap.reason
            )
        })
        .collect()
}

/// The defects as a prompt for a coding agent.
pub(crate) fn fix_prompt(repo: &std::path::Path, report: &orchestrate::RunReport) -> String {
    bugsleuth_engine::handoff::prompt(
        &repo.display().to_string(),
        &report.ranked,
        &gap_lines(report),
        report.swept.len(),
    )
}

/// Attempt proof for the top defects, if the settings ask for it.
///
/// Returns the section to append to the report, or nothing when proving is off.
/// Proving needs a test command and a git repository — a worktree cannot be made
/// without one — so a request that cannot be honoured says so rather than
/// failing silently.
/// Which model attempts proofs.
///
/// Taken from the models the user configured rather than hardcoded, so proving
/// does not silently depend on one vendor being installed. Claude and Codex can
/// both do it; Kilo cannot be constrained to a schema, so it is skipped rather
/// than attempted and reported as unattempted if it is all there is.
///
/// Falls back to Claude's cheapest model when nothing configured can prove, so
/// the feature degrades to "tries anyway and says if it could not" rather than
/// to silence.
fn prover(settings: &Settings) -> String {
    settings
        .models
        .iter()
        .map(|m| m.id.trim())
        .find(|id| {
            !id.is_empty()
                && !matches!(
                    bugsleuth_engine::sweep::Vendor::parse(id).0,
                    bugsleuth_engine::sweep::Vendor::Kilo
                )
        })
        .unwrap_or("haiku")
        .to_string()
}

pub(crate) async fn prove_top(
    app: &tauri::AppHandle,
    settings: &Settings,
    repo: &std::path::Path,
    report: &orchestrate::RunReport,
) -> String {
    if settings.prove_top == 0 {
        return String::new();
    }
    // These read as one paragraph on screen, so the continuations matter: a
    // multi-line literal without them bakes the source indentation into the
    // message, and both of these shipped with a run of spaces mid-sentence.
    let command = settings.test_command.trim();
    if command.is_empty() {
        return "\n\nProof was requested but no test command was given, so nothing was \
                attempted. That is not the same as nothing being provable."
            .to_string();
    }
    if !repo.join(".git").exists() {
        return "\n\nProof was requested but this is not a git repository. A proof attempt \
                needs a throwaway worktree, so nothing was attempted."
            .to_string();
    }

    let _ = app.emit(
        "run-progress",
        serde_json::json!({
            "kind": "batch_started",
            "index": 1,
            "total": 1,
            "units": [format!("proving the top {} defects", settings.prove_top)],
        }),
    );

    let proved = orchestrate::proving::prove_top(
        &report.ranked,
        &orchestrate::proving::ProveOptions {
            repo,
            model: &prover(settings),
            test_command: command,
            top: settings.prove_top,
            max_turns: 40,
            timeout: Duration::from_secs(2700),
            test_timeout: Duration::from_secs(600),
            api_key: None,
        },
    )
    .await;

    orchestrate::proving::to_text(&proved, report.ranked.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ModelSetting;

    fn settings_with(ids: &[&str]) -> Settings {
        Settings {
            models: ids
                .iter()
                .map(|id| ModelSetting {
                    id: (*id).to_string(),
                    lanes: vec!["correctness".into()],
                    effort: String::new(),
                    passes: 1,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn proving_uses_a_vendor_the_user_actually_configured() {
        // It was hardcoded to Claude, so the strongest verification the tool
        // has silently depended on one vendor being installed.
        assert_eq!(
            prover(&settings_with(&["codex:gpt-5.6-codex"])),
            "codex:gpt-5.6-codex"
        );
        assert_eq!(prover(&settings_with(&["opus", "codex:"])), "opus");
    }

    #[test]
    fn a_vendor_that_cannot_prove_is_skipped_rather_than_attempted() {
        // Kilo cannot be constrained to a schema, so a proof attempt from it
        // could not be trusted to report accurately.
        assert_eq!(
            prover(&settings_with(&["kilo:some/model", "codex:"])),
            "codex:"
        );
    }

    #[test]
    fn nothing_provable_still_leaves_something_to_try() {
        // Degrading to "tries anyway and says if it could not" beats silence,
        // which is what a missing prover would otherwise produce.
        assert_eq!(prover(&settings_with(&["kilo:only"])), "haiku");
        assert_eq!(prover(&settings_with(&[])), "haiku");
    }
}
