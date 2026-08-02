//! The frontend's entire surface.
//!
//! Every command is a deserialize, a call into `bugsleuth-engine`, and a
//! serialize. Nothing here decides anything; if a body starts growing a
//! judgement, that judgement belongs in the engine where it can be tested
//! without a window.
//!
//! Arguments arrive from a webview and are untrusted. In particular a
//! repository path is a string the frontend chose, so it is canonicalized and
//! checked to be a real directory before anything acts on it — the frontend has
//! no filesystem permission of its own precisely so that this is the only door.

use std::path::PathBuf;
use std::time::Duration;

use bugsleuth_engine::{orchestrate, plan};
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::settings::{self, Settings};

/// Errors cross to the frontend as plain strings. The webview cannot act on a
/// typed error, and a message a person can read is worth more than a variant.
type CommandResult<T> = Result<T, String>;

#[derive(Serialize)]
pub struct VendorStatus {
    pub name: String,
    pub available: bool,
    pub detail: String,
}

/// Which provider CLIs can be started. Free — starts no model.
#[tauri::command]
pub async fn preflight() -> Vec<VendorStatus> {
    bugsleuth_engine::sweep::probe_all()
        .await
        .into_iter()
        .map(|(name, result)| match result {
            Ok(version) => VendorStatus {
                name: name.to_string(),
                available: true,
                detail: version,
            },
            Err(error) => VendorStatus {
                name: name.to_string(),
                available: false,
                detail: error,
            },
        })
        .collect()
}

#[tauri::command]
pub fn load_settings() -> Settings {
    settings::load()
}

#[tauri::command]
pub fn save_settings(settings: Settings) -> CommandResult<()> {
    settings::save(&settings).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct PlanPreview {
    /// One entry per (model x lane) that would run.
    pub units: Vec<String>,
    /// Lanes with no model assigned. Shown before the run, not after, because
    /// this is the one gap a person can still fix for free.
    pub uncovered: Vec<String>,
    /// How many rounds the run takes, given one invocation per vendor at a time.
    pub batches: usize,
}

/// Show what a run would do, without doing it.
#[tauri::command]
pub fn plan_run(settings: Settings) -> CommandResult<PlanPreview> {
    let plan = plan::plan(&to_config(&settings)).map_err(|e| e.to_string())?;
    Ok(PlanPreview {
        units: plan
            .units
            .iter()
            .map(|unit| format!("{} × {}", unit.model, unit.lane.title()))
            .collect(),
        uncovered: plan
            .uncovered
            .iter()
            .map(|l| l.title().to_string())
            .collect(),
        batches: plan.batches().len(),
    })
}

/// Start a run. Returns immediately; progress and the result arrive as events.
///
/// Spawned rather than awaited so the command does not hold the frontend for
/// the tens of minutes a real sweep takes.
#[tauri::command]
pub async fn start_run(app: tauri::AppHandle, settings: Settings) -> CommandResult<()> {
    let repo = checked_repo(&settings.repo)?;
    let plan = plan::plan(&to_config(&settings)).map_err(|e| e.to_string())?;
    let out_dir = run_output_dir(&repo);

    // Forward engine progress to the window as it happens. A run is tens of
    // minutes; a front end that only learns the outcome at the end shows a
    // spinner for half an hour.
    let (progress, mut events) = tokio::sync::mpsc::unbounded_channel();
    let forwarder = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let _ = forwarder.emit("run-progress", event);
        }
    });

    tauri::async_runtime::spawn(async move {
        let report = orchestrate::run(
            &plan,
            orchestrate::RunOptions {
                repo: &repo,
                scope: non_empty(&settings.scope),
                max_turns: 40,
                timeout: Duration::from_secs(2700),
                api_key: None,
                out_dir: Some(&out_dir),
                resume: settings.reuse_completed,
                triage_model: &settings.triage_model,
                progress: Some(progress),
            },
        )
        .await;

        let payload = match report {
            Ok(report) => {
                let mut text = report.to_text();
                // Honour the proof settings. Without this the UI would offer a
                // "prove top N" control that silently does nothing, which is
                // precisely the defect BugSleuth's own UX lane exists to catch.
                text.push_str(&prove_top(&app, &settings, &repo, &report).await);

                // The prompt is the thing that gets used, so it is written to
                // disk as well as handed to the window. A run is tens of
                // minutes and the window can be closed; losing the output to a
                // stray click would be the worst possible ending.
                let prompt = fix_prompt(&repo, &report);
                let saved = bugsleuth_engine::handoff::write_all(
                    &out_dir,
                    &repo.display().to_string(),
                    &report.ranked,
                    &gap_lines(&report),
                    report.swept.len(),
                )
                .ok();
                serde_json::json!({
                    "ok": true,
                    "text": text,
                    "prompt": prompt,
                    "promptPath": saved.map(|p| p.display().to_string()),
                    "promptCount": report.ranked.len(),
                    "findings": crate::payload::findings(&repo.display().to_string(), &report),
                })
            }
            Err(error) => serde_json::json!({ "ok": false, "text": error.to_string() }),
        };
        let _ = app.emit("run-finished", payload);
    });

    Ok(())
}

/// Every lane nobody reviewed, as one line each.
fn gap_lines(report: &orchestrate::RunReport) -> Vec<String> {
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
fn fix_prompt(repo: &std::path::Path, report: &orchestrate::RunReport) -> String {
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
async fn prove_top(
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
            model: "sonnet",
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

/// Native folder picker. The frontend has no filesystem permission, so this is
/// how a repository gets chosen.
#[tauri::command]
pub async fn pick_directory(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    rx.recv().ok().flatten().map(|folder| folder.to_string())
}

/// Turn stored settings into the engine's configuration.
fn to_config(settings: &Settings) -> plan::Config {
    plan::Config {
        models: settings
            .models
            .iter()
            .map(|m| plan::ModelPlan {
                id: m.id.clone(),
                lanes: m.lanes.clone(),
                effort: m.effort.clone(),
                passes: m.passes.max(1),
            })
            .collect(),
    }
}

/// Resolve and sanity-check a repository path from the frontend.
fn checked_repo(raw: &str) -> CommandResult<PathBuf> {
    if raw.trim().is_empty() {
        return Err("choose a repository first".to_string());
    }
    let path = PathBuf::from(raw.trim())
        .canonicalize()
        .map_err(|e| format!("cannot open {raw}: {e}"))?;
    if !path.is_dir() {
        return Err(format!("{raw} is not a directory"));
    }
    // `canonicalize` yields Windows' extended-length form, which git rejects.
    let text = path.to_string_lossy();
    Ok(match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path.clone(),
    })
}

/// Where a run's per-sweep JSON goes: beside the app's settings, keyed by the
/// repository name, so runs are findable and removable without hunting.
fn run_output_dir(repo: &std::path::Path) -> PathBuf {
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    settings::data_dir().join("runs").join(name)
}

fn non_empty(value: &str) -> Option<&str> {
    Some(value.trim()).filter(|v| !v.is_empty())
}

/// Exit the application for real.
///
/// The window's close button hides to the tray, so without this the only way
/// out is the tray menu — and if the tray ever fails to appear, there would be
/// no way out at all short of the task manager. A second, keyboard-reachable
/// exit is cheap insurance against an unrecoverable state.
#[tauri::command]
pub fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

/// Reveal the window once the frontend has mounted and themed itself.
#[tauri::command]
pub fn frontend_ready(app: tauri::AppHandle) {
    crate::reveal(&app);
    let _ = app.get_webview_window("main");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_repository_path_is_refused_with_something_actionable() {
        let error = checked_repo("   ").unwrap_err();
        assert!(error.contains("choose a repository"));
    }

    #[test]
    fn a_path_that_does_not_exist_is_refused_rather_than_used() {
        assert!(checked_repo("Z:/definitely/not/here").is_err());
    }

    #[test]
    fn a_real_directory_resolves_without_the_extended_length_prefix() {
        // git rejects the \\?\ form that canonicalize produces on Windows, and
        // every worktree operation would fail on it.
        let resolved = checked_repo(".").unwrap_or_default();
        assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn settings_map_onto_the_engine_config_without_losing_lanes() {
        let settings = Settings::default();
        let config = to_config(&settings);
        assert_eq!(config.models.len(), settings.models.len());
        let planned = plan::plan(&config).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            planned.uncovered.is_empty(),
            "the default settings should leave no lane unswept"
        );
    }

    #[test]
    fn the_fix_prompt_carries_the_repo_and_every_unreviewed_lane() {
        // The app-side half of the deliverable. It cannot be observed without a
        // window, so it is checked here instead: an agent handed this must learn
        // which repository it is working on and which lanes nobody looked at.
        // Omitting either turns an incomplete review into an apparently
        // complete one.
        let report = orchestrate::RunReport {
            ranked: vec![],
            triage: Default::default(),
            swept: vec![],
            gaps: vec![
                orchestrate::Gap {
                    lane: bugsleuth_domain::Lane::Security,
                    model: None,
                    reason: "no model is assigned to this lane".to_string(),
                },
                orchestrate::Gap {
                    lane: bugsleuth_domain::Lane::Ux,
                    model: Some("kilo:".to_string()),
                    reason: "the kilo CLI exited with code 1".to_string(),
                },
            ],
        };
        let prompt = fix_prompt(std::path::Path::new("C:/x/my-repo"), &report);

        assert!(prompt.contains("my-repo"), "the repository is not named");
        assert!(
            prompt.contains("Security"),
            "an unassigned lane is not listed"
        );
        assert!(
            prompt.contains("nobody"),
            "an unassigned lane has no owner shown"
        );
        assert!(prompt.contains("Ux") || prompt.contains("UX"));
        assert!(
            prompt.contains("kilo:"),
            "a failed sweep does not name its model"
        );
        assert!(
            prompt.contains("not evidence that they are clean"),
            "nothing warns that the unreviewed lanes are not known-good"
        );
    }
}
