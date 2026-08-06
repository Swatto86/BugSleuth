//! Collecting the refs that killed runs leave behind.
//!
//! Every proof attempt gets a throwaway branch, deleted on drop. A run that is
//! killed rather than dropped never reaches that, and because the names carry
//! the creating process's id nothing will ever reuse one — so the branch stays
//! in the user's own repository for good. They accumulate one per kill, in
//! silence, in a repository BugSleuth promised only to read.
//!
//! Sweeping them has one hard requirement: a branch that a *live* worktree is
//! standing on must never be deleted. Another BugSleuth may be running against
//! the same repository right now, and taking its branch out from under it is a
//! worse failure than the leak.
//!
//! Two things enforce that. The liveness filter below is the one that actually
//! does the work: a branch a worktree is standing on never reaches the delete
//! at all. Git's own refusal — `git branch -D` declines a branch checked out in
//! any worktree — is the backstop underneath it, and it only matters if the
//! filter is wrong.
//!
//! Neither is free to remove. Dropping the filter leans the whole guarantee on
//! git's refusal, which `update-ref -d` and unlinking `refs/heads/...` bypass
//! silently; deleting refs directly while keeping the filter loses the backstop.
//! Keep both, and delete branches only through `git branch -D`.
//!
//! A listing that cannot be obtained is read as "everything is live", so an
//! unreadable repository sweeps nothing rather than sweeping everything.

use std::collections::HashSet;
use std::path::Path;

use super::git;

/// The prefix every throwaway branch carries. Named once so the code that
/// creates them and the code that collects them cannot drift apart — a sweep
/// matching a prefix nothing uses deletes nothing and looks exactly like a
/// sweep with nothing to do.
pub(super) const PREFIX: &str = "bugsleuth/";

/// Delete every `bugsleuth/*` branch that no live worktree is using.
///
/// Best effort throughout, like the directory sweep beside it: failing to tidy
/// up is not a reason to refuse to start a review.
pub(super) fn remove_orphan_branches(repo: &Path) {
    // Which branches are currently checked out in a worktree. A listing that
    // fails means we cannot tell live from dead, and the safe reading of that
    // is that everything is live.
    let Ok(listing) = git(repo, &["worktree", "list", "--porcelain"]) else {
        return;
    };
    let live = checked_out(&listing);

    let Ok(branches) = git(
        repo,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    ) else {
        return;
    };
    for branch in branches.lines().map(str::trim) {
        if !ours(branch) || live.contains(branch) {
            continue;
        }
        // `-D`, not `-d`: these branches are throwaway by construction and are
        // never merged anywhere, so git's merged-check would refuse every one
        // of them. That is safe only because the name is ours and nothing live
        // is standing on it — both established above.
        let _ = git(repo, &["branch", "-D", branch]);
    }
}

/// Whether a branch is one this tool generated, and so is ours to delete.
///
/// The prefix alone is not enough. Nothing reserves `bugsleuth/` in someone
/// else's repository — a user working on this very project could reasonably
/// name a branch `bugsleuth/faster-sweeps` — and deleting a hand-made branch is
/// data loss, not tidying. So the whole generated shape has to match:
/// `bugsleuth/<label>-<pid>-<counter>`, where the last two segments are the
/// numbers `create` appends. A human name is vanishingly unlikely to end in
/// `-<digits>-<digits>`, and one that does is at least the shape of something
/// this tool made.
fn ours(branch: &str) -> bool {
    let Some(slug) = branch.strip_prefix(PREFIX) else {
        return false;
    };
    // Split from the right: the label itself may contain hyphens.
    let mut parts = slug.rsplitn(3, '-');
    let counter = parts.next().unwrap_or("");
    let pid = parts.next().unwrap_or("");
    let label = parts.next().unwrap_or("");
    !label.is_empty()
        && !counter.is_empty()
        && !pid.is_empty()
        && counter.bytes().all(|b| b.is_ascii_digit())
        && pid.bytes().all(|b| b.is_ascii_digit())
}

/// The short branch names a `git worktree list --porcelain` listing reports as
/// checked out.
///
/// Split out so the parsing can be tested without worktrees: the listing gives
/// `branch refs/heads/<name>`, and a worktree with a detached HEAD gives no
/// `branch` line at all.
fn checked_out(listing: &str) -> HashSet<&str> {
    listing
        .lines()
        .filter_map(|line| line.strip_prefix("branch "))
        .filter_map(|reference| reference.trim().strip_prefix("refs/heads/"))
        .collect()
}

#[cfg(test)]
#[path = "orphans/tests.rs"]
mod tests;
