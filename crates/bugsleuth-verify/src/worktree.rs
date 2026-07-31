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
        let _ = remove(repo, &path);
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
        let _ = remove(&self.repo, &self.path);
        let _ = git(&self.repo, &["branch", "-D", &self.branch]);
    }
}

fn remove(repo: &Path, path: &Path) -> Result<String, WorktreeError> {
    git(
        repo,
        &["worktree", "remove", "--force", &path.to_string_lossy()],
    )
}

fn git(cwd: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let output = Command::new("git")
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
    fn creating_a_worktree_outside_a_git_repository_is_refused() {
        let not_a_repo = std::env::temp_dir().join("bugsleuth-not-a-repo");
        let _ = std::fs::create_dir_all(&not_a_repo);
        let result = Worktree::create(&not_a_repo, "HEAD", "x");
        assert!(matches!(result, Err(WorktreeError::NotAGitRepo(_))));
    }
}
