//! Throwaway git worktrees.
//!
//! Some vendors (Kilo) cannot be constrained to read-only, so a sweep by one has
//! to be run somewhere it cannot alter the code it is reviewing. It must never be
//! *your* checkout. A worktree on a throwaway branch gives each isolated sweep
//! its own directory, sharing the object store so it is cheap, and leaves the
//! working tree untouched no matter what the model does.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const PREFIX: &str = "bugsleuth/";

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("`{0}` is not inside a git repository, so an isolated worktree cannot be created")]
    NotAGitRepo(String),
    #[error("git {operation} failed: {message}")]
    Git { operation: String, message: String },
    #[error("could not run git — is it installed and on PATH? ({0})")]
    GitMissing(String),
    #[error(
        "refusing to use worktree directory `{path}` because it resolves outside the repository \
         or is a link/reparse point"
    )]
    UnsafeWorktreeRoot { path: String },
}

/// A checkout that deletes itself.
///
/// Cleanup runs on drop, including on panic and on an early return from an
/// error, so a failed isolated sweep cannot leave a stray branch and directory
/// behind. Cleanup failures are deliberately swallowed: a leaked temporary
/// directory is a much smaller problem than masking the real error that caused
/// the unwind.
#[derive(Debug)]
pub struct Worktree {
    repo: PathBuf,
    path: PathBuf,
    branch: String,
}

/// Distinguishes worktrees made by one process, as the process id cannot.
///
/// Two runs in different processes get different ids; two worktrees inside one
/// process would not, and the orchestrator is free to grow a second isolated
/// sweep in flight. A counter costs nothing and removes the question.
static NEXT: AtomicU64 = AtomicU64::new(0);

impl Worktree {
    /// Create a worktree of `repo` at `commit`, on a new throwaway branch.
    pub fn create(repo: &Path, commit: &str, label: &str) -> Result<Self, WorktreeError> {
        // Resolve the repository to its real location once, up front. Every
        // path below is then built from a canonical root, so nothing that
        // follows can be redirected by a component the reviewed repository
        // controls. A path that does not exist at all is simply not a repo.
        let repo = repo
            .canonicalize()
            .map_err(|_| WorktreeError::NotAGitRepo(repo.display().to_string()))?;
        if !repo.join(".git").exists() {
            return Err(WorktreeError::NotAGitRepo(repo.display().to_string()));
        }

        // The reviewed repository controls `.bugsleuth-worktrees`, so a
        // committed symlink or Windows junction there could point our cleanup
        // and `git worktree add` at an attacker-chosen directory. Validate the
        // container before it is read, deleted from, or written to — the
        // deletion sink (`remove`, reached from here, `remove_orphans`, and
        // `Drop`) recursively removes whatever it is handed.
        let root = checked_worktree_root(&repo)?;

        // Unique per process. The path used to be `<slug>` alone, so two
        // BugSleuth runs against one repository chose the same directory and
        // branch — and the second one's "clear the wreckage" step deleted the
        // first one's *live* worktree, taking a test run and the minutes it had
        // cost with it. Nothing warned; the losing run simply started failing.
        let label = sanitize(label);
        let (branch, path) = loop {
            let slug = format!(
                "{}-{}-{}",
                label,
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let branch = format!("{PREFIX}{slug}");
            let path = root.join(&slug);
            if !path.exists()
                && git(
                    &repo,
                    &[
                        "show-ref",
                        "--verify",
                        "--quiet",
                        &format!("refs/heads/{branch}"),
                    ],
                )
                .is_err()
            {
                break (branch, path);
            }
        };

        // A run killed rather than dropped leaves a directory behind, and the
        // unique path above means nothing will ever reuse and clean it. Git is
        // the authority on which of them are still worktrees: anything under
        // our directory that it no longer lists is wreckage.
        remove_orphans(&repo);

        git(
            &repo,
            &[
                "worktree",
                "add",
                "--force",
                "-b",
                &branch,
                &git_arg(&path),
                commit,
            ],
        )?;

        Ok(Self { repo, path, branch })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Repo-relative paths the model changed, staged or not.
    pub fn changed_files(&self) -> Result<Vec<String>, WorktreeError> {
        let out = git(&self.path, &["status", "--porcelain", "-z"])?;
        Ok(porcelain_paths(&out))
    }

    /// Throw away every change the model made, back to the base commit.
    pub fn reset(&self) -> Result<(), WorktreeError> {
        git(&self.path, &["reset", "--hard", "HEAD"])?;
        git(&self.path, &["clean", "-fdq"])?;
        Ok(())
    }

    /// Apply a patch file. Used by the eval to put the real bug fix back and
    /// re-run the model's test against fixed code.
    pub fn apply_patch(&self, patch: &Path) -> Result<(), WorktreeError> {
        git(&self.path, &["apply", &git_arg(patch)])?;
        Ok(())
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        remove(&self.repo, &self.path);
        let _ = git(&self.repo, &["branch", "-D", &self.branch]);
    }
}

/// Delete a worktree directory and deregister it.
///
/// `git worktree remove` alone is not enough. If anything built inside the
/// worktree — and a sweep that runs commands may have — the resulting `target/`
/// paths exceed the Windows 260-character limit and git gives up with "Filename
/// too long", leaving the directory behind. That is not cosmetic: the leftovers
/// make the *reviewed repository* dirty and quietly litter a repository
/// BugSleuth promised not to modify.
///
/// So: ask git first, then delete whatever survives ourselves, using the
/// extended-length path form that lifts the limit, and prune git's registry.
fn remove(repo: &Path, path: &Path) {
    let _ = git(repo, &["worktree", "remove", "--force", &git_arg(path)]);
    if path.exists() {
        let _ = std::fs::remove_dir_all(long_path(path));
    }
    // Take the `.bugsleuth-worktrees` parent too once it is empty. Git does not
    // track empty directories so it is harmless, but leaving one behind still
    // means litter in a repository we promised not to touch, and `remove_dir`
    // refuses if anything is still in there.
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(long_path(parent));
    }
    // Whether or not the directory went, git's registry must not keep pointing
    // at it, or the next run cannot reuse the same worktree name.
    let _ = git(repo, &["worktree", "prune"]);
}

/// Windows' extended-length path form, which raises the 260-character limit.
/// A no-op elsewhere, and on paths that already carry the prefix.
/// Delete anything under our directory that git no longer calls a worktree.
///
/// Paths carry the creating process's id, so nothing else will ever reuse a
/// directory left behind by a run that was killed. Rather than guess whether
/// some other BugSleuth still owns one — process ids are reused, and checking
/// liveness portably is its own problem — this asks git, which knows exactly
/// which worktrees exist. Anything it does not list is wreckage.
///
/// Best effort throughout: failing to tidy up is not a reason to refuse to
/// start a review.
/// Validate the worktree container before anything reads from, deletes under,
/// or writes into it, and fail closed on anything that is not the real
/// `.bugsleuth-worktrees` directory beneath the canonical repository.
///
/// `repo` is expected to already be canonical. The container is repository
/// controlled, so a committed symlink or Windows junction there could redirect
/// `read_dir`, `remove_dir_all`, and `git worktree add` at a directory outside
/// the repository, which would then be deleted with the user's permissions.
/// A missing container is fine — git creates it. An existing one must be a
/// genuine directory (not a symlink or reparse point) whose resolved location
/// is exactly the expected child of the repository.
fn checked_worktree_root(repo: &Path) -> Result<PathBuf, WorktreeError> {
    let root = repo.join(".bugsleuth-worktrees");
    let unsafe_root = || WorktreeError::UnsafeWorktreeRoot {
        path: root.display().to_string(),
    };
    match std::fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(root.clone()),
        Err(_) => Err(unsafe_root()),
        Ok(metadata) => {
            let resolved = root.canonicalize().map_err(|_| unsafe_root())?;
            let expected = repo.join(".bugsleuth-worktrees");
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || is_reparse_point(&metadata)
                || resolved != expected
            {
                return Err(unsafe_root());
            }
            Ok(root.clone())
        }
    }
}

/// Whether a directory entry is a Windows reparse point (a junction, mount
/// point, or symlink). `is_symlink()` alone does not catch junctions, which
/// need no elevation to create and would otherwise pass the directory check.
/// Always false off Windows.
#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn remove_orphans(repo: &Path) {
    let ours = repo.join(".bugsleuth-worktrees");
    let Ok(entries) = std::fs::read_dir(&ours) else {
        return;
    };
    // The container is validated as a real directory, but its *contents* are
    // repository-controlled: the reviewed repository can commit any directory it
    // likes under `.bugsleuth-worktrees/`. Only a genuine leftover worktree may
    // be treated as ours and removed. Fail closed if the container will not
    // resolve.
    let Ok(container) = ours.canonicalize() else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Real directories only. A symlink or Windows junction reports a
        // directory when followed; recursing into either is how a deletion
        // escapes the container, so reject anything whose own metadata is a
        // link or reparse point.
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            continue;
        }
        // A deregistered worktree of ours, or repository-controlled content?
        // Only the former carries the `.git` file that `git worktree add` writes
        // at the worktree root, and git refuses to track any path containing a
        // `.git` component, so nothing the reviewed repository can commit
        // satisfies this check.
        if !path.join(".git").is_file() {
            continue;
        }
        // Belt and braces: the resolved location must stay inside the container,
        // or this is not our wreckage either. Compared canonically because git
        // reports forward slashes and its own spelling of the drive.
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&container) {
            continue;
        }
        // Liveness is re-checked against a FRESH listing immediately before
        // deletion. Snapshotting the list once up front would treat a worktree
        // another process registered after the snapshot as wreckage. A listing
        // that cannot be obtained means "do not delete", not "delete
        // everything".
        let still_live = git(repo, &["worktree", "list", "--porcelain", "-z"])
            .map(|listing| {
                paths::worktree_roots(&listing).into_iter().any(|known| {
                    Path::new(known)
                        .canonicalize()
                        .ok()
                        .is_some_and(|k| k == canonical)
                })
            })
            .unwrap_or(true);
        if !still_live {
            remove(repo, &path);
        }
    }
}

fn long_path(path: &Path) -> PathBuf {
    if !cfg!(windows) {
        return path.to_path_buf();
    }
    let text = path.to_string_lossy();
    if text.starts_with(r"\\?\") || !path.is_absolute() {
        return path.to_path_buf();
    }
    PathBuf::from(format!(r"\\?\{}", text.replace('/', "\\")))
}

mod paths;
use paths::{git_arg, porcelain_paths};
pub use paths::{git_path, worktree_roots};

fn git(cwd: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let output = crate::console::hide(&mut Command::new("git"))
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| WorktreeError::GitMissing(e.to_string()))?;

    if !output.status.success() {
        return Err(WorktreeError::Git {
            operation: args.first().unwrap_or(&"?").to_string(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Reduce a label to something safe in a branch name and a path.
fn sanitize(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    let slug: String = trimmed.chars().take(48).collect();
    if slug.is_empty() {
        "run".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
#[path = "worktree/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "worktree/ownership_tests.rs"]
mod ownership_tests;
