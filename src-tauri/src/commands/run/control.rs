//! The one lock that keeps a review, an apply and a clear from overlapping.
//!
//! Split from `run` at the hard line cap, along the seam already there: this is
//! the mutually-exclusive state machine and nothing else, and what is left in
//! `run` is starting a sweep and working out where it writes.

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
    #[cfg(test)]
    fn running(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| matches!(&*state, WorkState::Running(_)))
    }

    /// Whether a fix is being applied.
    #[cfg(test)]
    fn applying(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| matches!(&*state, WorkState::Applying))
    }

    /// Whether stored sweeps are being deleted.
    #[cfg(test)]
    fn clearing(&self) -> bool {
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
