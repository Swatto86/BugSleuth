//! Starting a run, stopping it, and working out where it writes.
//!
//! Split from `commands` at the hard line cap, along the seam already
//! there: everything here is about the lifetime of one review, and what is
//! left is small independent answers to questions the window asks. The lock
//! that keeps a review, an apply and a clear from overlapping went the same
//! way at the same cap, into [`control`].

use std::path::PathBuf;
use std::time::Duration;

use bugsleuth_engine::{orchestrate, plan};
use tauri::{Emitter, Manager};

use super::CommandResult;
use crate::outcome::{fix_prompt, gap_lines, prove_top};
use crate::settings::{self, Settings};

mod control;

/// Re-exported so every caller keeps its existing path: the lock moved out of
/// this file at the hard line cap, it did not change hands.
pub use control::RunControl;

#[tauri::command]
pub fn cancel_run(control: tauri::State<'_, RunControl>) {
    control.cancel_run();
}

/// Start a run. Returns immediately; progress and the result arrive as events.
///
/// Spawned rather than awaited so the command does not hold the frontend for
/// the tens of minutes a real sweep takes.
#[tauri::command]
pub async fn start_run(
    app: tauri::AppHandle,
    control: tauri::State<'_, RunControl>,
    settings: Settings,
) -> CommandResult<()> {
    let repo = checked_repo(&settings.repo)?;
    let plan = plan::plan(&to_config(&settings)).map_err(|e| e.to_string())?;
    let out_dir = run_output_dir(&repo);

    // A fresh signal per run: reusing one would let a cancel from a finished
    // run stop the next one before it started. Reserving the running state is
    // the one guard that a review reads the tree while an apply rewrites it —
    // done atomically here, after the fallible setup above and before anything
    // is spawned, so a concurrent apply or a second run cannot slip in and no
    // early return leaves the state reserved.
    let cancel = bugsleuth_engine::cancel::Cancel::new();
    control.try_start_run(cancel.clone())?;

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
                cancel: cancel.clone(),
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
                // A save that half-failed used to be swallowed by `.ok()`, so the
                // window said Finished over a handoff that was missing files. Both
                // a hard failure and a partial one are surfaced now.
                let (prompt_path, save_error) = match bugsleuth_engine::handoff::write_all(
                    &out_dir,
                    &repo.display().to_string(),
                    &report.ranked,
                    &gap_lines(&report),
                    report.swept.len(),
                ) {
                    Ok(written) => {
                        let warning = (!written.warnings.is_empty()).then(|| {
                            format!(
                                "The review finished, but some fix prompts were not saved: {}",
                                written.warnings.join("; ")
                            )
                        });
                        (Some(written.bundle.display().to_string()), warning)
                    }
                    Err(error) => (
                        None,
                        Some(format!(
                            "The review finished, but its fix prompt could not be saved to {}: {error}",
                            out_dir.display()
                        )),
                    ),
                };
                if let Some(warning) = &save_error {
                    text.push_str("\n\n");
                    text.push_str(warning);
                }
                serde_json::json!({
                    "ok": true,
                    "text": text,
                    "prompt": prompt,
                    "promptPath": prompt_path,
                    "saveError": save_error,
                    "findings": crate::payload::findings(&repo.display().to_string(), &report),
                })
            }
            Err(error) => serde_json::json!({ "ok": false, "text": error.to_string() }),
        };
        let _ = app.emit("run-finished", payload);
        // Cleared here, at the real end of the work — after the triage pass and
        // after the report and fix prompt are on disk. Clearing it when Stop was
        // pressed would reopen the window in which a tray Quit could kill the
        // process before any of that had been written.
        if let Some(control) = app.try_state::<RunControl>() {
            control.finish_run();
        }
    });

    Ok(())
}

/// Turn stored settings into the engine's configuration.
pub(super) fn to_config(settings: &Settings) -> plan::Config {
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
pub(super) fn checked_repo(raw: &str) -> CommandResult<PathBuf> {
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
pub(super) fn run_output_dir(repo: &std::path::Path) -> PathBuf {
    // The leaf name alone is not a key. Two checkouts of the same project — a
    // worktree beside the original, a clone under a different parent — share a
    // folder name, and sharing a run directory means resume hands one of them
    // the other's sweeps and the report states the wrong provenance.
    //
    // The full path decides, shortened to a hash so the directory name stays a
    // directory name, with the leaf kept in front so a person can still tell
    // which is which by looking.
    use std::hash::{Hash, Hasher};
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    repo.hash(&mut hasher);
    let key = format!("{}-{:016x}", name, hasher.finish());
    settings::data_dir().join("runs").join(key)
}

pub(super) fn non_empty(value: &str) -> Option<&str> {
    Some(value.trim()).filter(|v| !v.is_empty())
}
