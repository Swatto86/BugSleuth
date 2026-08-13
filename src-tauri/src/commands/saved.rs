//! Throwing away what a previous review left on disk.
//!
//! Sweeps are cached per repository so a run that dies at nine of twelve is not
//! a total loss, and reuse is on by default because paying twice for the same
//! sweep is the surprising outcome. The cost of that default is invisible: a
//! review of code that has since changed reads exactly like a review of the code
//! in front of you.
//!
//! It cost a real evening. Sweeps taken at 22:12 were reused for a run started
//! the following afternoon, hours after the defects they described had been
//! fixed — so the fix prompt described a repository that no longer existed, and
//! the model handed it correctly refused to change anything. The report was
//! right, the run was wasted, and nothing said why.
//!
//! Unticking reuse already forces fresh sweeps. This deletes the stored ones
//! outright, which is the version you can be sure of.

use std::path::Path;

use super::RunControl;
use super::run::{checked_repo, run_output_dir};
use crate::settings::{self, Settings};

/// What was thrown away, so the window can invalidate only that run's prompt.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleared {
    /// Files removed. Zero is a normal answer, not a failure.
    pub removed: usize,
    /// The prompt invalidated by this delete, whether or not it existed.
    pub prompt_path: String,
}

/// Delete every stored sweep and fix prompt for the chosen repository.
///
/// Refused while a run or an apply is in flight: a review writes into this
/// directory as it goes, and deleting underneath it would throw away sweeps
/// that had just been paid for.
#[tauri::command]
pub async fn clear_saved(
    control: tauri::State<'_, RunControl>,
    settings: Settings,
) -> Result<Cleared, String> {
    // Reserve the clearing state atomically first: a run or an apply writes
    // into this directory as it goes, and a check-then-delete could race one
    // that starts in the gap. Always release it once the delete returns,
    // including its error path, then hand back the delete's own result.
    control.try_start_clear()?;
    let outcome = clear(&settings);
    control.finish_clear();
    outcome
}

/// The whole of what the command does, minus the guard.
///
/// Split out so the delete can be exercised against a real directory in a test:
/// a command that removes files should not be shipped having only ever been
/// checked by reading it.
fn clear(settings: &Settings) -> Result<Cleared, String> {
    let repo = checked_repo(&settings.repo)?;
    let dir = run_output_dir(&repo)?;
    let prompt_path = dir.join("fix-prompt.md").display().to_string();

    // Belt and braces on a delete. `run_output_dir` builds this path itself, so
    // it is already inside the app's own data directory — but a future change to
    // how it is built must not turn this into a command that removes an
    // arbitrary directory chosen by a webview.
    let root = settings::data_dir().join("runs");
    if !dir.starts_with(&root) {
        return Err(format!(
            "refusing to delete {}: it is not inside {}",
            dir.display(),
            root.display()
        ));
    }

    let removed = count_files(&dir);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(Cleared {
            removed,
            prompt_path,
        }),
        // Nothing stored for this repository yet. Reported as an ordinary zero
        // rather than as an error: "there was nothing to delete" is the same
        // outcome the user asked for.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Cleared {
            removed: 0,
            prompt_path,
        }),
        Err(error) => Err(format!("could not clear {}: {error}", dir.display())),
    }
}

/// How many files are in `dir`, counted before it is removed.
fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_gets_cleared_is_inside_the_apps_own_data_directory() {
        // The guard above compares against this prefix, so if the two ever
        // disagree the command either refuses everything or deletes somewhere
        // it should not.
        let dir = run_output_dir(Path::new("C:/work/some-repo")).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            dir.starts_with(settings::data_dir().join("runs")),
            "{dir:?}"
        );
    }

    #[test]
    fn clearing_really_deletes_the_stored_sweeps_and_says_how_many() {
        // Against the real filesystem, through the real path builder. A command
        // that deletes files must have been watched deleting them.
        let repo = std::env::temp_dir().join(format!("bugsleuth-clear-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&repo);
        // Through `checked_repo`, exactly as the command does. Canonicalizing
        // by hand here seeded a *different* directory and the delete found
        // nothing: the run directory is keyed by a hash of the path string, so
        // `\\?\C:\…` and `C:\…` are two different repositories as far as this
        // is concerned. Both sides going through one function is what makes
        // them agree.
        let resolved = checked_repo(&repo.display().to_string()).unwrap_or_else(|e| panic!("{e}"));
        let stored = run_output_dir(&resolved).unwrap_or_else(|e| panic!("{e}"));
        let _ = std::fs::create_dir_all(&stored);
        for name in [
            "correctness-haiku.json",
            "fix-prompt.md",
            "fix-prompt-01.md",
        ] {
            let _ = std::fs::write(stored.join(name), "x");
        }

        let settings = Settings {
            repo: repo.display().to_string(),
            ..Default::default()
        };
        let cleared = clear(&settings).unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(cleared.removed, 3);
        assert_eq!(
            cleared.prompt_path,
            stored.join("fix-prompt.md").display().to_string()
        );
        assert!(!stored.exists(), "the directory is still there: {stored:?}");

        // And again, with nothing left: an ordinary zero, not an error. "There
        // was nothing to delete" is the same outcome the user asked for.
        let again = clear(&settings).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.removed, 0);

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_repository_that_does_not_exist_is_refused_rather_than_guessed_at() {
        // Otherwise a typo'd path hashes to some other directory, and this
        // deletes a different repository's sweeps.
        let settings = Settings {
            repo: "Z:/definitely/not/here".to_string(),
            ..Default::default()
        };
        assert!(clear(&settings).is_err());
    }

    #[test]
    fn counting_a_directory_that_is_not_there_is_zero_rather_than_a_panic() {
        let missing = std::env::temp_dir().join("bugsleuth-no-such-dir-xyz");
        assert_eq!(count_files(&missing), 0);
    }

    #[test]
    fn only_files_are_counted_so_a_subdirectory_does_not_inflate_the_number() {
        let dir = std::env::temp_dir().join(format!("bugsleuth-count-{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("inner"));
        let _ = std::fs::write(dir.join("a.json"), "{}");
        let _ = std::fs::write(dir.join("b.md"), "x");
        assert_eq!(count_files(&dir), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
