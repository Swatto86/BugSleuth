//! Starting a run, stopping it, and working out where it writes.
//!
//! Split from `commands` at the hard line cap, along the seam already
//! there: everything here is about the lifetime of one review, and what is
//! left is small independent answers to questions the window asks. The lock
//! that keeps a review, an apply and a clear from overlapping went the same
//! way at the same cap, into [`control`].

use std::path::PathBuf;

use bugsleuth_engine::plan;
use tauri::{Emitter, Manager};

use super::CommandResult;
use crate::settings::{self, Settings};

mod batch;
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
    let repositories = batch::prepare(&settings)?;
    let plan = plan::plan(&to_config(&settings)).map_err(|e| e.to_string())?;
    let cancel = bugsleuth_engine::cancel::Cancel::new();
    control.try_start_run(cancel.clone())?;
    crate::tray::work_started(&app, crate::tray::BackgroundWork::Review);
    tauri::async_runtime::spawn(async move {
        let payload = batch::execute(&app, repositories, plan, settings, cancel).await;
        crate::tray::work_finished(
            &app,
            crate::tray::BackgroundWork::Review,
            if payload["cancelled"].as_bool().unwrap_or(false) {
                crate::tray::Completion::Stopped
            } else if !payload["ok"].as_bool().unwrap_or(false) {
                crate::tray::Completion::Failed
            } else if !payload["complete"].as_bool().unwrap_or(false)
                || !payload["saveError"].is_null()
            {
                crate::tray::Completion::Incomplete
            } else {
                crate::tray::Completion::Succeeded
            },
        );
        if let Some(control) = app.try_state::<RunControl>() {
            control.finish_run();
        }
        let _ = app.emit("run-finished", payload);
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
                use_agents: m.use_agents,
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
    // `canonicalize` yields Windows' extended-length form, which git rejects;
    // a UNC network path must keep its `\\server\share` form rather than be
    // truncated to a relative `UNC\server\share`. One shared conversion.
    Ok(bugsleuth_engine::git_path(&path))
}

/// A stable 64-bit hash of a path (FNV-1a over its bytes).
///
/// Explicitly defined so the directory a run is saved under does not move when
/// the compiler changes: `DefaultHasher` promises no stable algorithm across
/// Rust releases, and this value is part of a persisted directory-name contract,
/// so a toolchain bump would have orphaned every saved run under a name nothing
/// recomputes to.
fn stable_path_hash(repo: &std::path::Path) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in repo.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The old, unstable hash — read only, to migrate directories that used it.
fn legacy_path_hash(repo: &std::path::Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    repo.hash(&mut hasher);
    hasher.finish()
}

fn run_dir_for(repo: &std::path::Path, hash: u64) -> PathBuf {
    // The leaf name is kept in front so a person can still tell which checkout is
    // which by looking; the hash is what actually distinguishes two checkouts
    // that share a folder name.
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    settings::data_dir()
        .join("runs")
        .join(format!("{name}-{hash:016x}"))
}

/// Where a run's per-sweep JSON goes: beside the app's settings, keyed by the
/// repository path, so runs are findable and removable without hunting.
///
/// Migrates a directory written under the old unstable hash to the stable name
/// the first time it is asked for, while the current compiler can still
/// reproduce the old value. A rename failure is surfaced, never silently
/// swallowed into an empty new directory.
pub(super) fn run_output_dir(repo: &std::path::Path) -> CommandResult<PathBuf> {
    let stable = run_dir_for(repo, stable_path_hash(repo));
    if stable.exists() {
        return Ok(stable);
    }
    let legacy = run_dir_for(repo, legacy_path_hash(repo));
    if legacy.exists() {
        std::fs::rename(&legacy, &stable).map_err(|error| {
            format!(
                "could not migrate saved runs for {} to a stable location: {error}",
                repo.display()
            )
        })?;
    }
    Ok(stable)
}

pub(super) fn non_empty(value: &str) -> Option<&str> {
    Some(value.trim()).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unc_verbatim_path() {
        // The shared conversion `checked_repo` uses: a UNC network path must
        // survive as `\\server\share`, not the relative `UNC\server\share` that
        // dropping only `\\?\` leaves. Pure string work, so run everywhere.
        assert_eq!(
            bugsleuth_engine::git_path(std::path::Path::new(r"\\?\UNC\server\share\repo"))
                .to_string_lossy(),
            r"\\server\share\repo"
        );
        // End-to-end on this platform: checked_repo must never hand back a path
        // still wearing the extended-length prefix git rejects.
        #[cfg(windows)]
        {
            let resolved = checked_repo(".").expect("the working directory is a directory");
            assert!(
                !resolved.to_string_lossy().starts_with(r"\\?\"),
                "checked_repo returned an extended-length path: {}",
                resolved.display()
            );
        }
    }

    #[test]
    fn the_stable_hash_is_a_fixed_contract_that_a_toolchain_bump_cannot_move() {
        // A fixed vector: if this value ever changes, saved runs would be
        // orphaned, so the change must be deliberate and this must be updated
        // with a migration rather than silently drift as DefaultHasher did.
        assert_eq!(
            stable_path_hash(std::path::Path::new("C:/work/bugsleuth")),
            0x948c_e64e_57b0_658c
        );
    }

    #[test]
    fn a_directory_under_the_old_hash_is_migrated_to_the_stable_name() {
        // Seed a run directory under the legacy hash and confirm its contents
        // are found at the stable path after one lookup — otherwise a toolchain
        // upgrade would leave every saved run behind.
        let repo = std::env::temp_dir().join(format!("bugsleuth-migrate-{}", std::process::id()));
        let legacy = run_dir_for(&repo, legacy_path_hash(&repo));
        let stable = run_dir_for(&repo, stable_path_hash(&repo));
        let _ = std::fs::remove_dir_all(&legacy);
        let _ = std::fs::remove_dir_all(&stable);
        std::fs::create_dir_all(&legacy).expect("seed legacy dir");
        std::fs::write(legacy.join("correctness-haiku.json"), "x").expect("seed a sweep");

        let resolved = run_output_dir(&repo).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resolved, stable);
        assert!(
            stable.join("correctness-haiku.json").is_file(),
            "the seeded sweep did not survive the migration"
        );
        assert!(!legacy.exists(), "the legacy directory was left behind");

        let _ = std::fs::remove_dir_all(&stable);
    }
}
