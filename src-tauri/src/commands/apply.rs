//! Handing a finished review to a model that will act on it.
//!
//! The prompt is read from disk rather than taken from the window. The window
//! holds a copy — it is what the Copy button gives you — but it arrives here as
//! a string a webview chose, and this is the one command whose argument becomes
//! instructions to an agent with write access. The file the run wrote is the
//! only source that cannot have been substituted on the way.

use std::time::Duration;

use tauri::{Emitter, Manager};

use super::RunControl;
use super::run::{checked_repo, run_output_dir};
use crate::settings::Settings;

mod report;
use report::describe;

/// How long one apply may take, and how many turns it gets.
///
/// Far more generous than a sweep, because the work is: reading each defect,
/// changing the code, writing a test and running it, for every defect in the
/// report. Being cut short is not a disaster — everything done so far is in git
/// and is reported — but it is a waste, so the ceiling is high enough that
/// hitting it means something is wrong rather than that the list was long.
const APPLY_TIMEOUT: Duration = Duration::from_secs(7200);
const APPLY_MAX_TURNS: u32 = 300;

/// Apply the last run's fix prompt with the chosen model.
///
/// Returns immediately; the result arrives as an `apply-finished` event, exactly
/// as a run does, because this takes minutes to hours.
#[tauri::command]
pub async fn apply_fixes(
    app: tauri::AppHandle,
    control: tauri::State<'_, RunControl>,
    settings: Settings,
) -> Result<(), String> {
    let repo = checked_repo(&settings.repo)?;
    let model = settings.apply_model.trim().to_string();
    if model.is_empty() {
        return Err("choose a provider and model to apply the fixes with".to_string());
    }

    let effort = settings.apply_effort.trim().to_string();
    let prompt_path = run_output_dir(&repo)?.join("fix-prompt.md");
    let prompt = std::fs::read_to_string(&prompt_path).map_err(|e| {
        format!(
            "no fix prompt for this repository at {}: {e}. Run a review first.",
            prompt_path.display()
        )
    })?;

    // Reserve the applying state atomically, after the fallible setup above and
    // immediately before spawning. A review reads the tree while this rewrites
    // it, so the two must never overlap — and a single check-then-set could let
    // a run start in the gap. Every early return before here has reserved
    // nothing, so none leaks the state.
    control.try_start_apply()?;
    crate::tray::work_started(&app, crate::tray::BackgroundWork::Apply);
    tauri::async_runtime::spawn(async move {
        let report = bugsleuth_engine::apply::apply(bugsleuth_engine::apply::ApplyRequest {
            repo: &repo,
            model: &model,
            effort: &effort,
            prompt: &prompt,
            timeout: APPLY_TIMEOUT,
            max_turns: APPLY_MAX_TURNS,
            push: settings.push_after_apply,
            tag: settings.tag_release_after_push,
        })
        .await;

        let payload = match report {
            Ok(report) => serde_json::json!({
                "ok": true,
                "text": describe(&report),
                "changed": report.changed_files,
            }),
            Err(error) => serde_json::json!({
                "ok": false,
                "text": error.to_string(),
                "changed": Vec::<String>::new(),
            }),
        };
        // Applying can run for a long time with the window closed to the tray,
        // so its completion is announced the same way a review's is.
        crate::tray::work_finished(
            &app,
            crate::tray::BackgroundWork::Apply,
            payload["ok"].as_bool().unwrap_or(false),
        );
        let _ = app.emit("apply-finished", payload);
        if let Some(control) = app.try_state::<RunControl>() {
            control.finish_apply();
        }
    });

    Ok(())
}
