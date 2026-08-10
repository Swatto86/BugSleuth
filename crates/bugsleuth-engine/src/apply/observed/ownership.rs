//! Which changed paths are the user's work and which are BugSleuth's own.
//!
//! Split from `observed.rs` at the hard line cap, along its own seam: that file
//! asks git what changed, this one answers the separate question of whose it is.
//! The distinction has to be right in both directions — counting our leftovers
//! as the user's refuses every apply until someone deletes a directory by hand,
//! and counting theirs as ours silently discards work nobody was told about.

use std::path::{Path, PathBuf};

use super::git;

/// Where BugSleuth's own throwaway checkouts live inside a reviewed repository.
///
/// An isolated sweep (Kilo) that was killed rather than dropped leaves one
/// behind, and git reports it as untracked. Judged as the user's uncommitted
/// work it would refuse every apply — with advice to "commit or stash" litter
/// this tool left — until someone deleted a directory by hand.
pub(in crate::apply) const OURS: &str = ".bugsleuth-worktrees/";

/// The paths that are the user's work.
///
/// Ownership is what git currently has registered, not what a path is called.
/// Dropping every path under the container by prefix threw away the reviewed
/// repository's own files: the directory is repository-controlled and may hold
/// tracked or uncommitted user work, which then vanished from the clean-tree
/// guard and from the changed-file report alike — so the model's edits could be
/// mixed with, or overwrite, work nobody was ever shown.
///
/// Only a path inside a directory git reports as a secondary worktree is ours.
/// If the listing or a canonicalization fails, nothing is filtered: unexplained
/// debris under the container conservatively blocks the apply, which is the safe
/// direction when the alternative is silently discarding somebody's work.
pub(in crate::apply) fn theirs(repo: &Path, files: Vec<String>) -> Vec<String> {
    let registered = registered_worktrees(repo);
    files
        .into_iter()
        .filter(|path| !ours(repo, path, registered.as_deref()))
        .collect()
}

/// The canonical root of every worktree git currently registers for `repo`.
///
/// `None` when the listing could not be obtained or a registered path could not
/// be resolved — the caller must then filter nothing. The listing is read
/// NUL-delimited and untrimmed through the one parser that knows how, because a
/// repository path may contain a newline or significant whitespace.
fn registered_worktrees(repo: &Path) -> Option<Vec<PathBuf>> {
    let listing = git(repo, &["worktree", "list", "--porcelain", "-z"]).ok()?;
    bugsleuth_verify::worktree_roots(&listing)
        .into_iter()
        .map(|root| Path::new(root).canonicalize().ok())
        .collect()
}

/// Whether one repo-relative path lies inside a registered secondary worktree.
fn ours(repo: &Path, path: &str, registered: Option<&[PathBuf]>) -> bool {
    let Some(registered) = registered else {
        return false;
    };
    // The container's immediate child is the worktree root; anything deeper
    // belongs to it, and the container itself is never a worktree.
    let Some(rest) = path.strip_prefix(OURS) else {
        return false;
    };
    let Some(child) = rest.split('/').next().filter(|name| !name.is_empty()) else {
        return false;
    };
    let Ok(canonical) = repo.join(OURS).join(child).canonicalize() else {
        return false;
    };
    registered.contains(&canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real registered worktree is ours; anything else under the container
    /// belongs to the repository.
    ///
    /// Discarding the whole container by prefix threw away tracked user files
    /// committed under it: the apply started as though the tree were clean, and
    /// the model's edits could be mixed with — or overwrite — uncommitted work
    /// that never appeared in `changed_files` either.
    #[test]
    fn user_files_under_the_worktree_container_are_not_hidden() {
        let dir = std::env::temp_dir()
            .join("bugsleuth-container")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !run(&["init", "-q"]) {
            return; // no usable git here
        }
        let _ = run(&["config", "user.email", "t@example.invalid"]);
        let _ = run(&["config", "user.name", "test"]);
        std::fs::create_dir_all(dir.join(".bugsleuth-worktrees")).expect("container");
        std::fs::write(dir.join(".bugsleuth-worktrees/config.toml"), "keep = 1\n").expect("write");
        let _ = run(&["add", "-A"]);
        let _ = run(&["commit", "-qm", "base"]);

        let mixed = vec![
            ".bugsleuth-worktrees/config.toml".to_string(),
            "src/real.rs".to_string(),
        ];
        assert_eq!(
            theirs(&dir, mixed),
            [".bugsleuth-worktrees/config.toml", "src/real.rs"],
            "a tracked user file under the container was silently dropped"
        );

        // A genuinely registered worktree is still ours and still filtered.
        let held = bugsleuth_verify::Worktree::create(&dir, "HEAD", "container-test")
            .expect("create worktree");
        let child = held
            .path()
            .file_name()
            .expect("worktree name")
            .to_string_lossy()
            .to_string();
        assert_eq!(
            theirs(
                &dir,
                vec![
                    format!(".bugsleuth-worktrees/{child}/src/main.rs"),
                    ".bugsleuth-worktrees/config.toml".to_string(),
                ]
            ),
            [".bugsleuth-worktrees/config.toml"],
            "a real registered worktree's contents leaked into the user's work"
        );
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With no listing to compare against, nothing is anybody's but the user's.
    #[test]
    fn an_unreadable_worktree_listing_filters_nothing() {
        let missing = std::path::Path::new("definitely-not-a-repository-here");
        assert_eq!(
            theirs(missing, vec![".bugsleuth-worktrees/x/src/a.rs".to_string()]),
            [".bugsleuth-worktrees/x/src/a.rs"],
            "debris we cannot account for must block the apply, not vanish"
        );
    }
}
