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
use super::run::listed_repositories;
use crate::settings::{self, Settings};

/// What was thrown away, so the window can invalidate only those runs' prompts.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleared {
    /// Files removed, across every repository. Zero is a normal answer, not a
    /// failure.
    pub removed: usize,
    /// How many repositories were cleared, so the window can say "across N
    /// repositories" rather than leave the user wondering which were included.
    pub repositories: usize,
    /// The prompts invalidated by this delete, whether or not they existed.
    pub prompt_paths: Vec<String>,
}

/// Delete every stored sweep and fix prompt for every listed repository.
///
/// The whole list, not the first line: this became a multi-repository tool and
/// a clear that quietly left the other folders' sweeps in place meant their
/// next review reused stale sweeps while the button said it had wiped them.
///
/// Refused while a run or an apply is in flight: a review writes into these
/// directories as it goes, and deleting underneath it would throw away sweeps
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
    // The same resolution a run uses, so a folder that would be refused for a
    // review (missing, or nested inside another) is refused here too — before
    // anything is deleted, since one bad line must not cost the good ones
    // their sweeps or leave the list half cleared.
    let repositories = listed_repositories(settings)?;

    // Belt and braces on a delete. `run_output_dir` builds these paths itself,
    // so they are already inside the app's own data directory — but a future
    // change to how it is built must not turn this into a command that removes
    // an arbitrary directory chosen by a webview.
    let root = settings::data_dir().join("runs");
    for (_, dir) in &repositories {
        if !dir.starts_with(&root) {
            return Err(format!(
                "refusing to delete {}: it is not inside {}",
                dir.display(),
                root.display()
            ));
        }
    }

    let mut removed = 0;
    let mut prompt_paths = Vec::with_capacity(repositories.len());
    for (cleared, (_, dir)) in repositories.iter().enumerate() {
        prompt_paths.push(dir.join("fix-prompt.md").display().to_string());
        removed += remove_run_dir(dir).map_err(|error| {
            format!(
                "{error}. {cleared} of {} repositories had already been cleared",
                repositories.len()
            )
        })?;
    }
    Ok(Cleared {
        removed,
        repositories: repositories.len(),
        prompt_paths,
    })
}

/// Remove one repository's run directory, answering how many files went.
fn remove_run_dir(dir: &Path) -> Result<usize, String> {
    let removed = count_files(dir);
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(removed),
        // Nothing stored for this repository yet. Reported as an ordinary zero
        // rather than as an error: "there was nothing to delete" is the same
        // outcome the user asked for.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(format!("could not clear {}: {error}", dir.display())),
    }
}

/// What a reset threw away.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reset {
    /// Files removed across every run directory.
    pub removed: usize,
    /// Run directories removed — one per repository ever reviewed.
    pub repositories: usize,
}

/// Forget every run: delete every stored sweep, report and fix prompt for
/// every repository BugSleuth has ever reviewed, listed or not.
///
/// The clear above is scoped to the list, which is the right tool while the
/// list is the thing being worked on. This answers a different question —
/// "what is this report at the bottom of my window?" — where the honest
/// answer is a window that shows nothing until something is run. Settings are
/// untouched: the repository list, the model matrix and the theme are choices,
/// not results.
///
/// Refused while a run or an apply is in flight, for the reason given on
/// [`clear_saved`].
#[tauri::command]
pub async fn reset_saved(control: tauri::State<'_, RunControl>) -> Result<Reset, String> {
    control.try_start_clear()?;
    let outcome = reset(&settings::data_dir().join("runs"));
    control.finish_clear();
    outcome
}

/// The whole of what the reset does, minus the guard, against `runs`.
///
/// Takes the root rather than reading it from settings so the test can point
/// it at a scratch directory: a test that reset the real one would delete the
/// developer's own reports every time the suite ran.
fn reset(runs: &Path) -> Result<Reset, String> {
    let entries = match std::fs::read_dir(runs) {
        Ok(entries) => entries,
        // Nothing has ever been run. The outcome the user asked for.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Reset {
                removed: 0,
                repositories: 0,
            });
        }
        Err(error) => return Err(format!("could not list {}: {error}", runs.display())),
    };
    let mut removed = 0;
    let mut repositories = 0;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not list {}: {error}", runs.display()))?;
        let path = entry.path();
        if path.is_dir() {
            repositories += 1;
            removed += remove_run_dir(&path)?;
        } else {
            std::fs::remove_file(&path)
                .map_err(|error| format!("could not remove {}: {error}", path.display()))?;
            removed += 1;
        }
    }
    Ok(Reset {
        removed,
        repositories,
    })
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
    use crate::commands::run::{checked_repo, run_output_dir};
    use std::path::PathBuf;

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

    /// A repository folder plus the run directory the app would use for it,
    /// seeded with `files` so there is something to watch being deleted.
    ///
    /// Through `checked_repo`, exactly as the command does. Canonicalizing by
    /// hand here seeded a *different* directory and the delete found nothing:
    /// the run directory is keyed by a hash of the path string, so `\\?\C:\…`
    /// and `C:\…` are two different repositories as far as this is concerned.
    /// Both sides going through one function is what makes them agree.
    fn seeded(name: &str, files: &[&str]) -> (PathBuf, PathBuf) {
        let repo = std::env::temp_dir().join(format!("bugsleuth-{name}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&repo);
        let resolved = checked_repo(&repo.display().to_string()).unwrap_or_else(|e| panic!("{e}"));
        let stored = run_output_dir(&resolved).unwrap_or_else(|e| panic!("{e}"));
        let _ = std::fs::create_dir_all(&stored);
        for file in files {
            let _ = std::fs::write(stored.join(file), "x");
        }
        (repo, stored)
    }

    #[test]
    fn clearing_really_deletes_the_stored_sweeps_and_says_how_many() {
        // Against the real filesystem, through the real path builder. A command
        // that deletes files must have been watched deleting them.
        let (repo, stored) = seeded(
            "clear",
            &[
                "correctness-haiku.json",
                "fix-prompt.md",
                "fix-prompt-01.md",
            ],
        );

        let settings = Settings {
            repo: repo.display().to_string(),
            ..Default::default()
        };
        let cleared = clear(&settings).unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(cleared.removed, 3);
        assert_eq!(cleared.repositories, 1);
        assert_eq!(
            cleared.prompt_paths,
            [stored.join("fix-prompt.md").display().to_string()]
        );
        assert!(!stored.exists(), "the directory is still there: {stored:?}");

        // And again, with nothing left: an ordinary zero, not an error. "There
        // was nothing to delete" is the same outcome the user asked for.
        let again = clear(&settings).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.removed, 0);

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn every_listed_repository_is_cleared_not_only_the_first() {
        // The defect: the list grew to sixteen folders and the button kept
        // clearing line one, so the others' next review reused sweeps the user
        // believed were gone.
        let (first, first_stored) = seeded("clear-first", &["correctness-haiku.json"]);
        let (second, second_stored) =
            seeded("clear-second", &["security-haiku.json", "fix-prompt.md"]);
        // A folder with nothing stored yet is an ordinary zero, not a reason to
        // stop before the ones after it.
        let (third, third_stored) = seeded("clear-third", &[]);
        let _ = std::fs::remove_dir_all(&third_stored);

        let settings = Settings {
            repo: first.display().to_string(),
            additional_repos: vec![second.display().to_string(), third.display().to_string()],
            ..Default::default()
        };
        let cleared = clear(&settings).unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(cleared.removed, 3);
        assert_eq!(cleared.repositories, 3);
        assert_eq!(
            cleared.prompt_paths,
            [&first_stored, &second_stored, &third_stored]
                .map(|dir| dir.join("fix-prompt.md").display().to_string())
        );
        assert!(!first_stored.exists(), "still there: {first_stored:?}");
        assert!(!second_stored.exists(), "still there: {second_stored:?}");

        for repo in [first, second, third] {
            let _ = std::fs::remove_dir_all(repo);
        }
    }

    #[test]
    fn a_reset_forgets_every_run_whether_or_not_it_is_listed() {
        // Against a scratch root, never the real one: this deletes everything
        // under it, and the other tests here seed the real run directory.
        let runs = std::env::temp_dir().join(format!("bugsleuth-reset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runs);
        for (name, files) in [
            (
                "listed-repo-1111",
                &["correctness-haiku.json", "last-report.json"][..],
            ),
            (
                "forgotten-repo-2222",
                &["security-haiku.json", "fix-prompt.md", "fix-prompt-01.md"][..],
            ),
        ] {
            let dir = runs.join(name);
            std::fs::create_dir_all(&dir).expect("seed run dir");
            for file in files {
                std::fs::write(dir.join(file), "x").expect("seed file");
            }
        }

        let done = reset(&runs).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(done.repositories, 2);
        assert_eq!(done.removed, 5);
        assert_eq!(
            std::fs::read_dir(&runs)
                .map(|entries| entries.count())
                .unwrap_or(0),
            0,
            "a run directory survived the reset"
        );

        // Again with nothing left, and again with no root at all: both are the
        // outcome that was asked for, not errors.
        let again = reset(&runs).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((again.removed, again.repositories), (0, 0));
        let _ = std::fs::remove_dir_all(&runs);
        let never = reset(&runs).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((never.removed, never.repositories), (0, 0));
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
    fn one_unresolvable_line_refuses_the_clear_before_anything_is_deleted() {
        // Every line is resolved first. Deleting the good ones and then failing
        // on the bad one would report an error over a list that was half
        // cleared, with no way to tell which half.
        let (repo, stored) = seeded("clear-partial", &["correctness-haiku.json"]);
        let settings = Settings {
            repo: repo.display().to_string(),
            additional_repos: vec!["Z:/definitely/not/here".to_string()],
            ..Default::default()
        };
        assert!(clear(&settings).is_err());
        assert!(
            stored.exists(),
            "the valid repository's sweeps were deleted anyway"
        );
        // Its own debris: the refusal is the point, so nothing cleared it.
        let _ = std::fs::remove_dir_all(&stored);
        let _ = std::fs::remove_dir_all(&repo);
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
