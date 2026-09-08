//! Atomic reservations for reviews, fixes, clearing and updates.
//! Fixes may overlap only in separate, non-nested repository directories.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

enum WorkState {
    Idle,
    Running(bugsleuth_engine::cancel::Cancel),
    Applying(BTreeMap<PathBuf, (PathBuf, bugsleuth_engine::cancel::Cancel)>),
    Clearing,
    Updating,
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
            WorkState::Applying(_) => Err(
                "fixes are being applied to this repository — wait for that to finish".to_string(),
            ),
            WorkState::Clearing => {
                Err("saved sweeps are being cleared — wait for that to finish".to_string())
            }
            WorkState::Updating => {
                Err("an update is being installed — wait for it to finish".to_string())
            }
        }
    }

    /// Reserve the applying state, or say why it cannot start.
    pub fn try_start_apply(
        &self,
        repo: &Path,
        cancel: bugsleuth_engine::cancel::Cancel,
    ) -> Result<(), String> {
        let common = common_git_dir(repo).unwrap_or_else(|| repo.to_path_buf());
        let mut state = self.state.lock().map_err(|_| lock_poisoned())?;
        match &mut *state {
            WorkState::Idle => {
                *state =
                    WorkState::Applying(BTreeMap::from([(repo.to_path_buf(), (common, cancel))]));
                Ok(())
            }
            WorkState::Running(_) => Err(
                "a review is running — applying fixes now would edit the code it is reading"
                    .to_string(),
            ),
            WorkState::Applying(jobs) => {
                if jobs.iter().any(|(active, (git, _))| {
                    repo.starts_with(active) || active.starts_with(repo) || git == &common
                }) {
                    return Err("fixes are already being applied in this repository or an overlapping folder".into());
                }
                if jobs.len() >= 16 {
                    return Err("wait for one of the 16 active fixes to finish".into());
                }
                jobs.insert(repo.to_path_buf(), (common, cancel));
                Ok(())
            }
            WorkState::Clearing => {
                Err("saved sweeps are being cleared — wait for that to finish".to_string())
            }
            WorkState::Updating => {
                Err("an update is being installed — wait for it to finish".to_string())
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
            WorkState::Applying(_) => Err(
                "fixes are being applied — they are writing here, so wait for that to finish"
                    .to_string(),
            ),
            WorkState::Clearing => Err("saved sweeps are already being cleared".to_string()),
            WorkState::Updating => {
                Err("an update is being installed — wait for it to finish".to_string())
            }
        }
    }

    /// Reserve the updating state, or say which operation must finish first.
    pub fn try_start_update(&self) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| lock_poisoned())?;
        match &*state {
            WorkState::Idle => {
                *state = WorkState::Updating;
                Ok(())
            }
            WorkState::Running(_) => Err("a review is running — wait for it to finish".to_string()),
            WorkState::Applying(_) => {
                Err("fixes are being applied — wait for them to finish".to_string())
            }
            WorkState::Clearing => {
                Err("saved sweeps are being cleared — wait for that to finish".to_string())
            }
            WorkState::Updating => Err("an update is already being installed".to_string()),
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
    pub fn finish_apply(&self, repo: &Path) {
        if let Ok(mut state) = self.state.lock()
            && let WorkState::Applying(jobs) = &mut *state
        {
            jobs.remove(repo);
            if jobs.is_empty() {
                *state = WorkState::Idle;
            }
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

    /// Release the updating state after a rejected or failed installation.
    pub fn finish_update(&self) {
        if let Ok(mut state) = self.state.lock()
            && matches!(&*state, WorkState::Updating)
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
    pub fn applying(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| matches!(&*state, WorkState::Applying(_)))
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

    /// Stop the apply in flight, if there is one.
    pub fn cancel_apply(&self) {
        if let Ok(state) = self.state.lock()
            && let WorkState::Applying(jobs) = &*state
        {
            for (_, cancel) in jobs.values() {
                cancel.stop();
            }
        }
    }
}

// Linked worktrees share refs, tags and publication state: reserve them together.
fn common_git_dir(repo: &Path) -> Option<PathBuf> {
    let dot_git = repo.join(".git");
    if dot_git.is_dir() {
        return dot_git.canonicalize().ok();
    }
    let link = std::fs::read_to_string(dot_git).ok()?;
    let git = repo.join(link.trim().strip_prefix("gitdir: ")?);
    let common = std::fs::read_to_string(git.join("commondir")).ok()?;
    git.join(common.trim()).canonicalize().ok()
}

fn lock_poisoned() -> String {
    "internal error: the run-state lock was poisoned".to_string()
}

#[cfg(test)]
mod tests;
