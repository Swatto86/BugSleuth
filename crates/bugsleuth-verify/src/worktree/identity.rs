//! Confirming a directory's `.git` is its own, not a pointer at another repo.
//!
//! Both isolated-worktree creation and apply run git against a directory the
//! user chose, and git follows a `.git` file or link to decide which repository
//! it is operating on. An existence check on `.git` is therefore not an
//! authorisation check: a directory whose `.git` points at a victim repository
//! passes it, after which commits, refs and config are read from and written to
//! the victim while the provider sees the attacker's directory. The check uses
//! git's own resolution and the linked-worktree round trip to tell the two apart.

use std::path::Path;

use super::{WorktreeError, git};

/// Reject a directory whose `.git` does not belong to it.
///
/// `repo` need not be canonical; it is resolved here so both apply (which hands
/// in a raw path) and worktree creation (which canonicalises first) share one
/// authority. A directory that is not a git repository, or whose git directory
/// resolves to a different repository, is [`WorktreeError::NotAGitRepo`].
pub fn validate_repository_identity(repo: &Path) -> Result<(), WorktreeError> {
    let fail = || WorktreeError::NotAGitRepo(repo.display().to_string());
    let repo = repo.canonicalize().map_err(|_| fail())?;

    let output =
        git(&repo, &["rev-parse", "--show-toplevel", "--git-common-dir"]).map_err(|_| fail())?;
    let mut lines = output.lines();
    let Some(toplevel_raw) = lines.next() else {
        return Err(fail());
    };
    let Some(common_raw) = lines.next() else {
        return Err(fail());
    };

    // The reported top-level must be exactly this directory. Git reports
    // forward slashes; canonicalize makes the comparison platform-honest.
    let toplevel = Path::new(toplevel_raw.trim())
        .canonicalize()
        .map_err(|_| fail())?;
    if toplevel != repo {
        return Err(fail());
    }

    // `--git-common-dir` may be absolute or relative to the working tree.
    let common = Path::new(common_raw.trim());
    let common = if common.is_absolute() {
        common.to_path_buf()
    } else {
        repo.join(common)
    };
    let common = common.canonicalize().map_err(|_| fail())?;

    if common_belongs_to(&repo, &common) {
        Ok(())
    } else {
        Err(fail())
    }
}

/// Whether `common` is genuinely this repository's git directory.
///
/// The standard layout has `common == repo/.git`. A linked worktree instead
/// keeps its per-worktree git dir under an ancestor repository's `.git`, reached
/// through a `.git` *file* whose target records the round trip back here — and
/// that round trip is exactly what a `.git` file aimed at an unrelated repository
/// cannot produce: its target is the victim's common dir directly, which holds
/// no `gitdir` pointing back.
fn common_belongs_to(repo: &Path, common: &Path) -> bool {
    if common == repo.join(".git") {
        return true;
    }
    let Some(target) = gitdir_target(repo.join(".git")) else {
        return false;
    };
    let target = if target.is_absolute() {
        target
    } else {
        repo.join(target)
    };
    let Ok(per_worktree) = target.canonicalize() else {
        return false;
    };
    // The per-worktree git dir must live under the common dir, and its own
    // `gitdir` file must name this repository's `.git` right back.
    if !per_worktree.starts_with(common) {
        return false;
    }
    let Some(back) = gitdir_target(per_worktree.join("gitdir")) else {
        return false;
    };
    let own_git = repo.join(".git");
    Path::new(&back)
        .canonicalize()
        .is_ok_and(|resolved| resolved == own_git.canonicalize().unwrap_or(own_git))
}

/// The path named by a `gitdir: <path>` pointer file, if that is what this is.
fn gitdir_target(pointee: impl AsRef<Path>) -> Option<std::path::PathBuf> {
    let text = std::fs::read_to_string(pointee.as_ref()).ok()?;
    text.trim()
        .strip_prefix("gitdir:")
        .map(str::trim)
        .map(std::path::PathBuf::from)
}
