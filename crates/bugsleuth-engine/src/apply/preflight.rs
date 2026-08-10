//! Where the repository stands before the model is allowed to touch it.
//!
//! Split from `apply.rs` at the hard line cap, along the seam already implied by
//! the ordering there: everything in this file runs before any quota is spent
//! and can only refuse, while the rest of `apply` decides what to run and reads
//! back what happened. Both questions here are about the *checked-out branch*,
//! which is not the same question as one about the repository as a whole.

use std::path::Path;

use super::Baseline;
use super::observed::{dirty_files, git, summarise, theirs};

/// Refuse before spending anything if the user has uncommitted work.
///
/// Their work and the model's would be mixed together with no way to revert one
/// without the other. `--untracked-files=all` so git reports the individual
/// children of an untracked directory rather than collapsing it to the
/// directory name: with the whole of `.bugsleuth-worktrees` named once, no child
/// path could be matched against the worktrees git actually has registered, and
/// a user file committed under there would be discarded with our own leftovers.
pub(super) fn refuse_if_dirty(repo: &Path) -> anyhow::Result<()> {
    let dirty = theirs(
        repo,
        dirty_files(&git(
            repo,
            &["status", "--porcelain", "-z", "--untracked-files=all"],
        )?),
    );
    if !dirty.is_empty() {
        anyhow::bail!(
            "the working tree has uncommitted changes, so applying fixes is refused: your work \
             and the model's would be mixed together with no way to revert one without the \
             other. Commit or stash these first — {}",
            summarise(&dirty)
        );
    }
    Ok(())
}

/// Delegates to [`current_head`], which answers about the checked-out branch.
pub(super) fn baseline(repo: &Path) -> anyhow::Result<Baseline> {
    Ok(match current_head(repo)? {
        Some(id) => Baseline::Commit(id),
        None => Baseline::Unborn,
    })
}

/// The commit the current branch points at, or `None` when that branch is
/// unborn.
///
/// `git rev-parse --verify HEAD` resolves to the current commit or fails when
/// the branch is unborn; `--quiet` keeps it from printing an error. A failure is
/// not automatically "unborn", though — git could be broken — so it is confirmed
/// with `symbolic-ref HEAD`, which succeeds exactly when HEAD names a branch
/// that simply has no commit yet.
///
/// The confirmation used to be `rev-list --all --count`, which asks about the
/// whole repository: a perfectly valid `git switch --orphan` branch has no HEAD
/// while other branches hold history, and every apply against it was refused as
/// a corrupt repository. Detached HEAD and damaged refs still error, because
/// neither answers this question either.
pub(super) fn current_head(repo: &Path) -> anyhow::Result<Option<String>> {
    if let Ok(out) = git(repo, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
        let id = out.trim().to_string();
        if !id.is_empty() {
            return Ok(Some(id));
        }
    }
    match git(repo, &["symbolic-ref", "--quiet", "HEAD"]) {
        Ok(reference) if !reference.trim().is_empty() => Ok(None),
        _ => anyhow::bail!(
            "could not resolve HEAD in {} and it does not name an unborn branch — is HEAD \
             detached or the repository corrupt?",
            repo.display()
        ),
    }
}
