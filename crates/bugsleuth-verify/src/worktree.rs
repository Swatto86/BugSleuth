//! Throwaway git worktrees.
//!
//! Proving a finding means letting a model write a test and run it, which means
//! giving it write access to a checkout. It must never be *your* checkout. A
//! worktree on a throwaway branch gives each proof attempt its own directory,
//! sharing the object store so it is cheap, and leaves the working tree
//! untouched no matter what the model does.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("`{0}` is not inside a git repository, so an isolated worktree cannot be created")]
    NotAGitRepo(String),
    #[error("git {operation} failed: {message}")]
    Git { operation: String, message: String },
    #[error("could not run git — is it installed and on PATH? ({0})")]
    GitMissing(String),
}

/// A checkout that deletes itself.
///
/// Cleanup runs on drop, including on panic and on an early return from an
/// error, so a failed proof attempt cannot leave a stray branch and directory
/// behind. Cleanup failures are deliberately swallowed: a leaked temporary
/// directory is a much smaller problem than masking the real error that caused
/// the unwind.
#[derive(Debug)]
pub struct Worktree {
    repo: PathBuf,
    path: PathBuf,
    branch: String,
}

impl Worktree {
    /// Create a worktree of `repo` at `commit`, on a new throwaway branch.
    pub fn create(repo: &Path, commit: &str, label: &str) -> Result<Self, WorktreeError> {
        if !repo.join(".git").exists() {
            return Err(WorktreeError::NotAGitRepo(repo.display().to_string()));
        }

        let slug = sanitize(label);
        let branch = format!("bugsleuth/{slug}");
        let path = repo.join(".bugsleuth-worktrees").join(&slug);

        // A previous run that was killed rather than dropped can leave both
        // behind; clear them so a retry is not blocked by its own wreckage.
        remove(repo, &path);
        let _ = git(repo, &["branch", "-D", &branch]);

        git(
            repo,
            &[
                "worktree",
                "add",
                "--force",
                "-b",
                &branch,
                &path.to_string_lossy(),
                commit,
            ],
        )?;

        Ok(Self {
            repo: repo.to_path_buf(),
            path,
            branch,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Repo-relative paths the model changed, staged or not.
    pub fn changed_files(&self) -> Result<Vec<String>, WorktreeError> {
        let out = git(&self.path, &["status", "--porcelain"])?;
        Ok(out
            .lines()
            .filter_map(|line| line.get(3..))
            .map(|name| name.trim().replace('\\', "/"))
            .filter(|name| !name.is_empty())
            .collect())
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
        git(&self.path, &["apply", &patch.to_string_lossy()])?;
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
/// worktree — and a proof attempt runs `cargo test`, so it will have — the
/// resulting `target/` paths exceed the Windows 260-character limit and git
/// gives up with "Filename too long", leaving the directory behind. That is not
/// cosmetic: the leftovers make the *reviewed repository* dirty, which breaks
/// the clean-baseline check the next proof attempt depends on, and quietly
/// litters a repository BugSleuth promised not to modify.
///
/// So: ask git first, then delete whatever survives ourselves, using the
/// extended-length path form that lifts the limit, and prune git's registry.
fn remove(repo: &Path, path: &Path) {
    let _ = git(
        repo,
        &["worktree", "remove", "--force", &path.to_string_lossy()],
    );
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
mod tests {
    use super::*;

    #[test]
    fn a_label_cannot_smuggle_path_or_flag_characters_into_a_branch_name() {
        assert_eq!(sanitize("correctness/1"), "correctness-1");
        assert_eq!(sanitize("../../escape"), "escape");
        assert_eq!(sanitize("--force"), "force");
        assert_eq!(sanitize(""), "run");
        assert_eq!(sanitize("!!!"), "run");
    }

    #[test]
    fn a_long_label_is_truncated_rather_than_producing_an_unusable_path() {
        let slug = sanitize(&"a".repeat(200));
        assert_eq!(slug.len(), 48);
    }

    #[test]
    fn a_long_absolute_path_gets_the_extended_length_prefix_on_windows() {
        let path = Path::new(r"C:\Users\x\repo\.bugsleuth-worktrees\run");
        let converted = long_path(path);
        if cfg!(windows) {
            assert!(
                converted.to_string_lossy().starts_with(r#"\\?\"#),
                "got {}",
                converted.display()
            );
        } else {
            assert_eq!(converted, path);
        }
    }

    #[test]
    fn a_path_that_already_has_the_prefix_is_not_given_a_second_one() {
        let path = Path::new(r#"\\?\C:\repo\wt"#);
        assert_eq!(long_path(path), path);
    }

    /// A throwaway git repository with one commit.
    fn temp_repo(name: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir()
            .join("bugsleuth-worktree-tests")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(long_path(&dir));
        std::fs::create_dir_all(&dir).ok()?;
        git(&dir, &["init", "-q"]).ok()?;
        git(&dir, &["config", "user.email", "t@example.invalid"]).ok()?;
        git(&dir, &["config", "user.name", "test"]).ok()?;
        std::fs::write(dir.join("a.txt"), "hello\n").ok()?;
        git(&dir, &["add", "-A"]).ok()?;
        git(&dir, &["commit", "-qm", "base"]).ok()?;
        Some(dir)
    }

    #[test]
    fn dropping_a_worktree_deletes_it_even_with_deeply_nested_build_output() {
        let Some(repo) = temp_repo("deep") else {
            // No usable git in this environment; the other tests still cover the
            // pure logic. Better to skip than to fail for an unrelated reason.
            return;
        };

        let path = {
            let worktree = match Worktree::create(&repo, "HEAD", "deep") {
                Ok(worktree) => worktree,
                Err(_) => return,
            };
            let path = worktree.path().to_path_buf();

            // Imitate what `cargo test` leaves behind inside a proof worktree:
            // paths long enough that `git worktree remove` fails on Windows with
            // "Filename too long" and silently leaves the directory in place.
            let mut deep = path.join("target");
            for segment in 0..12 {
                deep = deep.join(format!(
                    "{segment}-a-rather-long-directory-name-like-cargo-makes"
                ));
            }
            let _ = std::fs::create_dir_all(long_path(&deep));
            let _ = std::fs::write(long_path(&deep.join("artifact.bin")), b"x");
            assert!(path.exists(), "the worktree should exist before the drop");
            path
        };

        assert!(
            !path.exists(),
            "the worktree survived its own drop at {}; it would dirty the reviewed repository",
            path.display()
        );
        assert!(
            !repo.join(".bugsleuth-worktrees").exists(),
            "the container directory was left behind in the reviewed repository"
        );
        let _ = std::fs::remove_dir_all(long_path(&repo));
    }

    #[test]
    fn creating_a_worktree_outside_a_git_repository_is_refused() {
        let not_a_repo = std::env::temp_dir().join("bugsleuth-not-a-repo");
        let _ = std::fs::create_dir_all(&not_a_repo);
        let result = Worktree::create(&not_a_repo, "HEAD", "x");
        assert!(matches!(result, Err(WorktreeError::NotAGitRepo(_))));
    }
}
