//! Starting a run, stopping it, and working out where it writes.
//!
//! Split from `commands` at the hard line cap, along the seam already
//! there: everything here is about the lifetime of one review, and what is
//! left is small independent answers to questions the window asks.

use std::path::PathBuf;
use std::time::Duration;

use bugsleuth_engine::{orchestrate, plan};
use tauri::{Emitter, Manager};

use super::CommandResult;
use crate::outcome::{fix_prompt, gap_lines, prove_top};
use crate::settings::{self, Settings};

/// The single record of what mutually-exclusive work is in flight, held for
/// the app's life.
///
/// Running, applying, and clearing must never overlap: a sweep reads the tree
/// while an apply rewrites it, so the report would describe code that no longer
/// exists, and clearing deletes the very sweeps a run is writing. The window
/// disables the buttons, but the window is not the only way in and a disabled
/// button is not a lock.
///
/// It used to be two independent primitives — an `Option<Cancel>` for the run
/// and an `AtomicBool` for the apply — each checked and then set as separate
/// steps. Two commands could both read idle before either recorded its work,
/// and a second run could overwrite the first's cancel signal. One
/// mutex-protected state instead, so every idle-to-active transition is a
/// single atomic step under one lock.
enum WorkState {
    Idle,
    Running(bugsleuth_engine::cancel::Cancel),
    Applying,
    Clearing,
}

pub struct RunControl {
    state: std::sync::Mutex<WorkState>,
}

impl Default for RunControl {
    fn default() -> Self {
        Self {
            state: std::sync::Mutex::new(WorkState::Idle),
        }
    }
}

/// Stop the run in progress.
///
/// Sweeps already written to disk are kept, so pressing Run again with reuse
/// enabled picks up from where this left off rather than paying twice.
impl RunControl {
    /// Reserve the running state for a fresh sweep, or say why it cannot start.
    ///
    /// The `Cancel` is stored so [`RunControl::cancel_run`] stops this exact run
    /// and no other, and a fresh one per run means a stopped run's signal cannot
    /// cancel the next. One atomic transition: two commands racing here cannot
    /// both observe idle, and a second run is refused rather than silently
    /// overwriting the first's signal.
    pub fn try_start_run(&self, cancel: bugsleuth_engine::cancel::Cancel) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| lock_poisoned())?;
        match &*state {
            WorkState::Idle => {
                *state = WorkState::Running(cancel);
                Ok(())
            }
            WorkState::Running(_) => Err("a review is already running".to_string()),
            WorkState::Applying => Err(
                "fixes are being applied to this repository — wait for that to finish".to_string(),
            ),
            WorkState::Clearing => {
                Err("saved sweeps are being cleared — wait for that to finish".to_string())
            }
        }
    }

    /// Reserve the applying state, or say why it cannot start.
    pub fn try_start_apply(&self) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| lock_poisoned())?;
        match &*state {
            WorkState::Idle => {
                *state = WorkState::Applying;
                Ok(())
            }
            WorkState::Running(_) => Err(
                "a review is running — applying fixes now would edit the code it is reading"
                    .to_string(),
            ),
            WorkState::Applying => Err("fixes are already being applied".to_string()),
            WorkState::Clearing => {
                Err("saved sweeps are being cleared — wait for that to finish".to_string())
            }
        }
    }

    /// Reserve the clearing state, or say why it cannot start.
    pub fn try_start_clear(&self) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| lock_poisoned())?;
        match &*state {
            WorkState::Idle => {
                *state = WorkState::Clearing;
                Ok(())
            }
            WorkState::Running(_) => Err(
                "a review is running — it is writing here, so wait for it to finish".to_string(),
            ),
            WorkState::Applying => Err(
                "fixes are being applied — they are writing here, so wait for that to finish"
                    .to_string(),
            ),
            WorkState::Clearing => Err("saved sweeps are already being cleared".to_string()),
        }
    }

    /// Mark the run over, however it ended — after the background task has
    /// written everything it is going to write, not when cancellation is
    /// requested, which is the middle of the work rather than the end of it.
    ///
    /// Clears only the running state, so a stray call cannot idle an apply or
    /// clear that started afterwards.
    pub fn finish_run(&self) {
        if let Ok(mut state) = self.state.lock()
            && matches!(&*state, WorkState::Running(_))
        {
            *state = WorkState::Idle;
        }
    }

    /// Mark an apply over. Clears only the applying state.
    pub fn finish_apply(&self) {
        if let Ok(mut state) = self.state.lock()
            && matches!(&*state, WorkState::Applying)
        {
            *state = WorkState::Idle;
        }
    }

    /// Mark a clear over. Clears only the clearing state.
    pub fn finish_clear(&self) {
        if let Ok(mut state) = self.state.lock()
            && matches!(&*state, WorkState::Clearing)
        {
            *state = WorkState::Idle;
        }
    }

    /// Whether a sweep is in flight. Present exactly while the task is alive.
    pub fn running(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| matches!(&*state, WorkState::Running(_)))
    }

    /// Whether a fix is being applied.
    pub fn applying(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| matches!(&*state, WorkState::Applying))
    }

    /// Whether stored sweeps are being deleted. Short-lived, but killing it
    /// mid-delete is still worth a prompt, so the tray counts it as busy.
    pub fn clearing(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| matches!(&*state, WorkState::Clearing))
    }

    /// Stop the run in flight, if there is one.
    pub fn cancel_run(&self) {
        if let Ok(state) = self.state.lock()
            && let WorkState::Running(cancel) = &*state
        {
            cancel.stop();
        }
    }
}

fn lock_poisoned() -> String {
    "internal error: the run-state lock was poisoned".to_string()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_engine::cancel::Cancel;
    use std::sync::{Arc, Barrier};
    use std::thread;

    /// Race two idle-to-active transitions against one shared, idle control and
    /// report which won, as `(first_ok, second_ok)`. A `Barrier` lines the two
    /// threads up so both attempt the transition at once.
    fn race(
        first: impl Fn(&RunControl) -> Result<(), String> + Send + 'static,
        second: impl Fn(&RunControl) -> Result<(), String> + Send + 'static,
    ) -> (Arc<RunControl>, bool, bool) {
        let control = Arc::new(RunControl::default());
        let barrier = Arc::new(Barrier::new(2));

        let c1 = Arc::clone(&control);
        let b1 = Arc::clone(&barrier);
        let t1 = thread::spawn(move || {
            b1.wait();
            first(&c1).is_ok()
        });
        let c2 = Arc::clone(&control);
        let b2 = Arc::clone(&barrier);
        let t2 = thread::spawn(move || {
            b2.wait();
            second(&c2).is_ok()
        });
        let first_ok = t1.join().unwrap();
        let second_ok = t2.join().unwrap();
        (control, first_ok, second_ok)
    }

    #[test]
    fn run_control_run_and_apply_cannot_both_start() {
        for _ in 0..200 {
            let (control, run_ok, apply_ok) =
                race(|c| c.try_start_run(Cancel::new()), |c| c.try_start_apply());
            assert!(run_ok ^ apply_ok, "exactly one of run/apply must win");
            if run_ok {
                assert!(control.running() && !control.applying());
            } else {
                assert!(control.applying() && !control.running());
            }
        }
    }

    #[test]
    fn run_control_only_one_of_two_runs_can_start() {
        for _ in 0..200 {
            let (control, first, second) = race(
                |c| c.try_start_run(Cancel::new()),
                |c| c.try_start_run(Cancel::new()),
            );
            assert!(first ^ second, "a second run started over the first");
            assert!(control.running());
        }
    }

    #[test]
    fn run_control_clear_and_run_cannot_both_start() {
        for _ in 0..200 {
            let (control, clear_ok, run_ok) =
                race(|c| c.try_start_clear(), |c| c.try_start_run(Cancel::new()));
            assert!(clear_ok ^ run_ok, "clear and run both started");
            assert!(control.running() || control.clearing());
        }
    }

    #[test]
    fn finishing_clears_only_its_own_state() {
        // A stray completion from one operation must not idle another that
        // started after it.
        let control = RunControl::default();
        control
            .try_start_apply()
            .expect("apply should start from idle");
        control.finish_run();
        assert!(control.applying(), "finish_run wrongly idled a live apply");
        control.finish_apply();
        assert!(!control.running() && !control.applying() && !control.clearing());
    }
}
