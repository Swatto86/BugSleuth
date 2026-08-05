//! What git says the model actually did.
//!
//! Split from `apply.rs` at the hard line cap, along the seam that was already
//! there: that file decides what to run and refuses when it must not, and this
//! one answers the only questions that matter afterwards — what changed, what
//! was committed, and whether any of it credits a tool the repository's owner
//! did not agree to credit.
//!
//! Nothing here takes the model's word for anything. That is the point.

use std::path::Path;
use std::process::Command;

/// Repo-relative paths differing from `base`, committed or not.
///
/// Two questions, because one command answers neither on its own: `git diff`
/// knows about tracked files only, and a new file the model created is exactly
/// the kind of change most worth seeing.
pub(super) fn changed_since(repo: &Path, base: Option<&str>) -> Vec<String> {
    let mut files: Vec<String> = match base {
        Some(base) => git(repo, &["diff", "--name-only", base])
            .map(|out| lines(&out))
            .unwrap_or_default(),
        // No commit to compare against — a repository with no history yet. The
        // working tree is all there is.
        None => git(repo, &["status", "--porcelain"])
            .map(|out| dirty_files(&out))
            .unwrap_or_default(),
    };
    files.extend(
        git(repo, &["ls-files", "--others", "--exclude-standard"])
            .map(|out| lines(&out))
            .unwrap_or_default(),
    );
    files.sort();
    files.dedup();
    theirs(files)
}

pub(super) fn commits_since(repo: &Path, base: Option<&str>) -> usize {
    let Some(base) = base else { return 0 };
    git(repo, &["rev-list", "--count", &format!("{base}..HEAD")])
        .ok()
        .and_then(|out| out.trim().parse().ok())
        .unwrap_or(0)
}

/// Where BugSleuth's own throwaway checkouts live inside a reviewed repository.
///
/// A proof attempt that was killed rather than dropped leaves one behind, and
/// git reports it as untracked. Judged as the user's uncommitted work it would
/// refuse every apply — with advice to "commit or stash" litter this tool left —
/// until someone deleted a directory by hand. It is not their work, so it is not
/// counted as theirs, in the guard or in the list of what changed.
const OURS: &str = ".bugsleuth-worktrees/";

pub(super) fn theirs(files: Vec<String>) -> Vec<String> {
    files
        .into_iter()
        .filter(|path| !path.starts_with(OURS))
        .collect()
}

/// The paths in `git status --porcelain` output.
///
/// The status letters occupy the first two columns and a space follows, so the
/// name starts at column three. A rename is reported as `old -> new`; the new
/// name is the one that exists, and it is what a reader needs.
pub(super) fn dirty_files(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|name| {
            let name = name.rsplit(" -> ").next().unwrap_or(name);
            name.trim().trim_matches('"').replace('\\', "/")
        })
        .filter(|name| !name.is_empty())
        .collect()
}

pub(super) fn lines(out: &str) -> Vec<String> {
    out.lines()
        .map(|line| line.trim().replace('\\', "/"))
        .filter(|line| !line.is_empty())
        .collect()
}

/// A few names and a count, rather than a wall of paths in an error message.
pub(super) fn summarise(files: &[String]) -> String {
    const SHOWN: usize = 5;
    let head = files
        .iter()
        .take(SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    match files.len().checked_sub(SHOWN) {
        Some(rest) if rest > 0 => format!("{head} and {rest} more"),
        _ => head,
    }
}

pub(super) fn git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    git_with_env(repo, args, &[])
}

/// `git`, with extra environment. Used to carry an original commit's author and
/// dates onto its rewrite, which `commit-tree` would otherwise stamp with this
/// process's identity and the time of day.
pub(super) fn git_with_env(
    repo: &Path,
    args: &[&str],
    env: &[(String, String)],
) -> anyhow::Result<String> {
    let mut command = Command::new("git");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = bugsleuth_verify::hide_console_window(&mut command)
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| anyhow::anyhow!("could not run git — is it installed and on PATH? ({e})"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&"?"),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dirty_tree_is_read_out_of_git_status_whatever_the_status_letters_are() {
        let porcelain = " M src/main.rs\n?? new.txt\nA  added.rs\nR  old.rs -> renamed.rs\n";
        let dirty = dirty_files(porcelain);
        assert_eq!(dirty, ["src/main.rs", "new.txt", "added.rs", "renamed.rs"]);
    }

    #[test]
    fn a_clean_tree_reads_as_clean_rather_than_as_one_empty_path() {
        // The guard is `is_empty()`, so a blank line surviving the parse would
        // refuse to apply anything on a perfectly clean repository.
        assert!(dirty_files("").is_empty());
        assert!(dirty_files("\n\n").is_empty());
        // A status line with the letters and nothing after them. The name is
        // then the empty string, and one of those in the list is enough to
        // refuse to apply anything on a repository that is perfectly clean.
        assert!(dirty_files("?? \n").is_empty());
    }

    #[test]
    fn bugsleuths_own_leftovers_are_not_treated_as_the_users_uncommitted_work() {
        // A proof attempt that was killed leaves a worktree behind, and git
        // calls it untracked. Counting it would refuse every apply — telling
        // the user to commit or stash a directory this tool created — until
        // they deleted it by hand.
        let mixed = vec![
            ".bugsleuth-worktrees/prove-1/src/main.rs".to_string(),
            "src/real.rs".to_string(),
        ];
        assert_eq!(theirs(mixed), ["src/real.rs"]);
        // And a repository whose only "changes" are ours reads as clean.
        assert!(theirs(vec![".bugsleuth-worktrees/x".to_string()]).is_empty());
    }

    #[test]
    fn the_refusal_names_a_few_files_rather_than_all_of_them() {
        let many: Vec<String> = (0..9).map(|n| format!("f{n}.rs")).collect();
        let text = summarise(&many);
        assert!(text.contains("f0.rs"));
        assert!(text.contains("and 4 more"), "{text}");
        assert!(!text.contains("f8.rs"));
        assert_eq!(summarise(&["one.rs".to_string()]), "one.rs");
    }
}
