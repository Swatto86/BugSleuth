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

    // Reserve the applying state *before* reading anything the run owns. A
    // review reads the tree while this rewrites it, so the two must never
    // overlap — and a single check-then-set could let a run start in the gap.
    //
    // The prompt used to be read first. Clear saved sweeps could then take the
    // state lock in between, delete the run directory and release it, after
    // which this reserved an idle state and edited the repository from a prompt
    // that had already been deleted: the operations linearize as clear before
    // apply, while apply consumes pre-clear state. Repository and model checks
    // stay above because they touch nothing shared.
    let cancel = bugsleuth_engine::cancel::Cancel::new();
    let prompt = reserve_and_load(&control, &repo, cancel.clone())?;
    crate::tray::work_started(&app, crate::tray::BackgroundWork::Apply);
    tauri::async_runtime::spawn(async move {
        let request = bugsleuth_engine::apply::apply(bugsleuth_engine::apply::ApplyRequest {
            repo: &repo,
            model: &model,
            effort: &effort,
            prompt: &prompt,
            timeout: APPLY_TIMEOUT,
            max_turns: APPLY_MAX_TURNS,
            cancel: cancel.clone(),
            push: settings.push_after_apply,
            tag: settings.tag_release_after_push,
        });
        // Cancellation is carried out of the select rather than folded into the
        // error, which made a deliberate Stop indistinguishable from a provider
        // failure — the apply's `ok: false` said only that there was no report.
        let (report, cancelled) = tokio::select! {
            biased;
            () = cancel.cancelled() => (
                Err(anyhow::anyhow!(
                    "the apply was stopped. The model was killed part-way through editing the repository — check `git status` and `git log` to see what it had already changed."
                )),
                true,
            ),
            report = request => (report, false),
        };

        let payload = match report {
            Ok(report) => serde_json::json!({
                "ok": true,
                "cancelled": cancelled,
                "text": describe(&report),
                "changed": report.changed_files,
            }),
            Err(error) => serde_json::json!({
                "ok": false,
                "cancelled": cancelled,
                "text": error.to_string(),
                "changed": Vec::<String>::new(),
            }),
        };
        // Applying can run for a long time with the window closed to the tray,
        // so its completion is announced the same way a review's is.
        crate::tray::work_finished(
            &app,
            crate::tray::BackgroundWork::Apply,
            if cancelled {
                crate::tray::Completion::Stopped
            } else if payload["ok"].as_bool().unwrap_or(false) {
                crate::tray::Completion::Succeeded
            } else {
                crate::tray::Completion::Failed
            },
        );
        let _ = app.emit("apply-finished", payload);
        if let Some(control) = app.try_state::<RunControl>() {
            control.finish_apply();
        }
    });

    Ok(())
}

/// Take the applying state, then read the prompt it protects.
///
/// One function so there is no order to get wrong. Reading first left a window
/// in which Clear saved sweeps could take the state lock, delete the run
/// directory and release it — after which the apply reserved an idle state and
/// edited the repository from a prompt that had already been deleted. The two
/// operations linearized as clear-before-apply while apply consumed pre-clear
/// state.
///
/// Every failure releases the reservation on the way out. Leaking it refuses
/// every later run, apply and clear until the app is restarted.
fn reserve_and_load(
    control: &RunControl,
    repo: &std::path::Path,
    cancel: bugsleuth_engine::cancel::Cancel,
) -> Result<String, String> {
    control.try_start_apply(cancel)?;
    load_prompt(repo).inspect_err(|_| control.finish_apply())
}

/// The fix prompt the last run wrote for this repository.
fn load_prompt(repo: &std::path::Path) -> Result<String, String> {
    let prompt_path = run_output_dir(repo)?.join("fix-prompt.md");
    std::fs::read_to_string(&prompt_path).map_err(|e| {
        format!(
            "no fix prompt for this repository at {}: {e}. Run a review first.",
            prompt_path.display()
        )
    })
}

/// Stop the apply in flight. The provider process is killed; commits it had
/// already made stay in git, and the `apply-finished` event says it was stopped.
#[tauri::command]
pub fn cancel_apply(control: tauri::State<'_, RunControl>) {
    control.cancel_apply();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reserving before loading is what makes the two operations order.
    ///
    /// Reading the prompt first left a window in which Clear saved sweeps could
    /// take the state lock, delete the run directory and release it — after
    /// which this reserved an idle state and edited the repository using a
    /// prompt that had already been deleted.
    #[test]
    fn clearing_cannot_start_between_the_reservation_and_the_prompt_load() {
        let dir = std::env::temp_dir()
            .join("bugsleuth-apply-order")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let output = run_output_dir(&dir).expect("run output directory");
        std::fs::create_dir_all(&output).expect("run directory");
        std::fs::write(output.join("fix-prompt.md"), "fix it\n").expect("prompt");

        let control = RunControl::default();
        let prompt = reserve_and_load(&control, &dir, bugsleuth_engine::cancel::Cancel::new())
            .expect("the prompt is there");
        assert_eq!(prompt, "fix it\n");
        // The state is already reserved on return, so no clear can have slipped
        // in between taking it and reading the directory it protects.
        assert!(
            control.try_start_clear().is_err(),
            "a clear could delete the run directory while apply was loading from it"
        );
        control.finish_apply();
        assert!(
            control.try_start_clear().is_ok(),
            "the reservation outlived the apply"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reservation must come first, not merely exist by the time this
    /// returns.
    ///
    /// Both orders leave the state reserved on return, so the behavioural test
    /// above cannot tell them apart — and the whole defect is the window
    /// *between* them. `reserve_and_load` is two statements precisely so this
    /// can be read; both anchors are asserted present so the scan cannot go
    /// vacuous if either call is renamed.
    #[test]
    fn the_applying_state_is_taken_before_the_run_directory_is_read() {
        let source = include_str!("apply.rs");
        let start = source
            .find("fn reserve_and_load(")
            .expect("reserve_and_load is gone; this check needs rewriting");
        let body = &source[start..];
        let body = &body[..body.find("\n}\n").map_or(body.len(), |end| end + 2)];
        let reserve = body
            .find("control.try_start_apply(")
            .expect("reserve_and_load no longer reserves the applying state");
        let load = body
            .find("load_prompt(repo)")
            .expect("reserve_and_load no longer reads the prompt");
        assert!(
            reserve < load,
            "the run directory is read before the state that protects it is taken, \
             so a clear can delete it in between"
        );
    }

    /// A failed load must not leave the app reserved for the session.
    #[test]
    fn a_failed_prompt_load_returns_the_state_to_idle() {
        let missing = std::env::temp_dir()
            .join("bugsleuth-apply-no-prompt")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        std::fs::create_dir_all(&missing).expect("scratch");

        let control = RunControl::default();
        let error = reserve_and_load(&control, &missing, bugsleuth_engine::cancel::Cancel::new())
            .expect_err("there is no prompt there");
        assert!(error.contains("Run a review first"), "{error}");
        assert!(
            control.try_start_clear().is_ok(),
            "a failed load left applying reserved, so clearing is refused forever"
        );
        let _ = std::fs::remove_dir_all(&missing);
    }
}
