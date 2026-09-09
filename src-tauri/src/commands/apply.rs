//! Handing a finished review to a model that will act on it.
//!
//! The prompt is read from disk rather than taken from the window. The window
//! holds a copy — it is what the Copy button gives you — but it arrives here as
//! a string a webview chose, and this is the one command whose argument becomes
//! instructions to an agent with write access. The file the run wrote is the
//! only source that cannot have been substituted on the way.

use std::future::Future;
use std::time::Duration;

use bugsleuth_engine::cancel::Cancel;
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

async fn await_engine_apply<F, T>(request: F, cancel: &Cancel) -> (T, bool)
where
    F: Future<Output = T>,
{
    let report = request.await;
    (report, cancel.stopped())
}

fn completion_for_apply(cancelled: bool, changed_files: Option<usize>) -> crate::tray::Completion {
    if cancelled {
        crate::tray::Completion::Stopped
    } else {
        match changed_files {
            None => crate::tray::Completion::Failed,
            Some(0) => crate::tray::Completion::NoChanges,
            Some(_) => crate::tray::Completion::Succeeded,
        }
    }
}

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
    let prompts = reserve_and_load(&control, &repo, cancel.clone())?;
    crate::tray::work_started(&app, crate::tray::BackgroundWork::Apply);
    tauri::async_runtime::spawn(async move {
        // Forwarded to the window as each defect starts and finishes. A fix run
        // is minutes to hours of a model editing a real repository; without
        // this the window shows one spinner for all of it, and stopping is a
        // decision made blind about how much would be thrown away.
        let (progress, mut events) = tokio::sync::mpsc::unbounded_channel();
        let forwarder = app.clone();
        let identity = repo.display().to_string();
        let forwarding = tauri::async_runtime::spawn(async move {
            while let Some(event) = events.recv().await {
                let mut payload = serde_json::json!(event);
                payload["repo"] = serde_json::json!(identity);
                let _ = forwarder.emit("apply-progress", payload);
            }
        });
        let request = bugsleuth_engine::apply::apply(bugsleuth_engine::apply::ApplyRequest {
            repo: &repo,
            model: &model,
            effort: &effort,
            prompts: &prompts,
            timeout: APPLY_TIMEOUT,
            max_turns: APPLY_MAX_TURNS,
            cancel: cancel.clone(),
            progress: Some(progress.clone()),
            push: settings.push_after_apply,
            tag: settings.tag_release_after_push,
        });
        // The engine owns cancellation and reconciles whatever git or the
        // remote accepted before it returns. Dropping that future here loses
        // the changed-file and uncertain-publication report.
        let (report, cancelled) = await_engine_apply(request, &cancel).await;
        drop(progress);
        let _ = forwarding.await;

        let (payload, changed_files) = match report {
            Ok(report) => {
                let changed_files = report.changed_files.len();
                (
                    serde_json::json!({
                        "repo": repo,
                        "model": model,
                        "ok": true,
                        "cancelled": cancelled,
                        "text": describe(&report),
                        "changed": report.changed_files,
                    }),
                    Some(changed_files),
                )
            }
            Err(error) => (
                serde_json::json!({
                    "repo": repo,
                    "model": model,
                    "ok": false,
                    "cancelled": cancelled,
                    "text": error.to_string(),
                    "changed": Vec::<String>::new(),
                }),
                None,
            ),
        };
        // Applying can run for a long time with the window closed to the tray,
        // so its completion is announced the same way a review's is.
        crate::tray::work_finished(
            &app,
            crate::tray::BackgroundWork::Apply,
            completion_for_apply(cancelled, changed_files),
        );
        if let Some(control) = app.try_state::<RunControl>() {
            control.finish_apply(&repo);
            if control.applying() {
                crate::tray::work_started(&app, crate::tray::BackgroundWork::Apply);
            }
        }
        let _ = app.emit("apply-finished", payload);
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
) -> Result<std::path::PathBuf, String> {
    control.try_start_apply(repo, cancel)?;
    load_prompt(repo).inspect_err(|_| control.finish_apply(repo))
}

/// Where the last run wrote this repository's fix prompts.
///
/// The directory, not the prompts. The engine reads the individual work orders
/// itself, because it applies them one defect at a time and because this is the
/// one command whose argument becomes instructions to an agent with write
/// access — the files the run wrote are the only source that cannot have been
/// substituted on the way here.
///
/// The bundle is still what is checked for. `handoff` writes it before any
/// per-defect prompt, so its absence is the honest "no review has produced a
/// fix prompt for this repository yet" — and answering that here keeps the app
/// out of the applying state instead of reserving it and failing a moment later.
fn load_prompt(repo: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let dir = run_output_dir(repo)?;
    let bundle = dir.join("fix-prompt.md");
    if !bundle.is_file() {
        return Err(format!(
            "no fix prompt for this repository at {}. Run a review first.",
            bundle.display()
        ));
    }
    Ok(dir)
}

/// How many defects an interrupted fix run for this repository already fixed.
///
/// Zero, and no entry at all, are different answers and both are `None` here:
/// there is nothing to resume in either case. The window uses this to offer a
/// resume rather than silently re-applying a whole report — which, after a run
/// that stopped on a quota limit, is what the Apply button would otherwise
/// appear to be doing.
#[tauri::command]
pub fn unfinished_apply(repo: String) -> Option<usize> {
    let repo = checked_repo(&repo).ok()?;
    bugsleuth_engine::apply::unfinished(&run_output_dir(&repo).ok()?)
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn a_successful_apply_that_changed_nothing_is_not_announced_as_applied() {
        assert_eq!(
            completion_for_apply(false, Some(0)),
            crate::tray::Completion::NoChanges
        );
        assert_eq!(
            completion_for_apply(false, Some(1)),
            crate::tray::Completion::Succeeded
        );
        assert_eq!(
            completion_for_apply(false, None),
            crate::tray::Completion::Failed
        );
        assert_eq!(
            completion_for_apply(true, Some(1)),
            crate::tray::Completion::Stopped
        );
    }

    #[tokio::test]
    async fn apply_cancellation_is_left_to_the_engine() {
        let cancel = bugsleuth_engine::cancel::Cancel::new();
        let request_cancel = cancel.clone();
        let reconciled = Arc::new(AtomicBool::new(false));
        let request_reconciled = Arc::clone(&reconciled);
        let request = async move {
            request_cancel.cancelled().await;
            request_reconciled.store(true, Ordering::Relaxed);
            "engine report"
        };
        cancel.stop();

        let (report, cancelled) = await_engine_apply(request, &cancel).await;

        assert_eq!(report, "engine report");
        assert!(reconciled.load(Ordering::Relaxed));
        assert!(cancelled);
    }

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
        let prompts = reserve_and_load(&control, &dir, bugsleuth_engine::cancel::Cancel::new())
            .expect("the prompt is there");
        assert_eq!(prompts, output);
        // The state is already reserved on return, so no clear can have slipped
        // in between taking it and reading the directory it protects.
        assert!(
            control.try_start_clear().is_err(),
            "a clear could delete the run directory while apply was loading from it"
        );
        control.finish_apply(&dir);
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
