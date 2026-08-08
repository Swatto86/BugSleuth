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

use anyhow::Context;

use super::Baseline;

/// Repo-relative paths differing from `base`, committed or not.
///
/// Two questions, because one command answers neither on its own: `git diff`
/// knows about tracked files only, and a new file the model created is exactly
/// the kind of change most worth seeing.
pub(super) fn changed_since(repo: &Path, base: &Baseline) -> anyhow::Result<Vec<String>> {
    let mut files: Vec<String> = match base {
        // A real starting commit: `diff` against it covers committed and
        // uncommitted changes to tracked files alike. A git failure here is not
        // "no changes" — a damaged `.git` must never read as a clean tree — so
        // it is propagated rather than swallowed to an empty list.
        Baseline::Commit(base) => lines(&git(repo, &["diff", "--name-only", base])?),
        // Started from no commit at all. If the model committed, everything now
        // in HEAD is new against the empty tree the repository began at; if it
        // did not, HEAD is still unborn and only the working-tree scans apply.
        // `range_since` tells those two apart from a broken repository.
        Baseline::Unborn => match range_since(repo, base)? {
            Some(_) => {
                let mut committed = lines(&git(repo, &["ls-tree", "-r", "--name-only", "HEAD"])?);
                committed.extend(dirty_files(&git(repo, &["status", "--porcelain"])?));
                committed
            }
            None => dirty_files(&git(repo, &["status", "--porcelain"])?),
        },
    };
    files.extend(lines(&git(
        repo,
        &["ls-files", "--others", "--exclude-standard"],
    )?));
    files.sort();
    files.dedup();
    Ok(theirs(files))
}

/// The `base..HEAD` revision range, or `None` when an unborn baseline still has
/// no commits — the model committed nothing, which is a genuine empty result
/// rather than a failure to inspect.
///
/// The two are told apart the way [`super::baseline`] tells them apart:
/// `rev-list --all --count` is `0` only when the repository truly has no
/// commits. Any other Git failure is propagated, never read as "nothing here" —
/// a damaged `.git` must not masquerade as a clean, empty history.
pub(super) fn range_since(repo: &Path, base: &Baseline) -> anyhow::Result<Option<String>> {
    match base {
        Baseline::Commit(base) => Ok(Some(format!("{base}..HEAD"))),
        Baseline::Unborn => {
            let all = git(repo, &["rev-list", "--all", "--count"])?;
            if all.trim() == "0" {
                Ok(None)
            } else {
                Ok(Some("HEAD".to_string()))
            }
        }
    }
}

pub(super) fn commits_since(repo: &Path, base: &Baseline) -> anyhow::Result<usize> {
    // Against a real commit, count the range past it. Against an unborn start
    // that reached a commit, every commit is new, so the whole of HEAD is the
    // count; an unborn start that committed nothing is a genuine zero. A git
    // failure is none of those — it means the count is unknown, so it is an
    // error rather than a silent zero that would read as "nothing to push".
    let Some(range) = range_since(repo, base)? else {
        return Ok(0);
    };
    let out = git(repo, &["rev-list", "--count", &range])?;
    out.trim()
        .parse()
        .with_context(|| format!("git returned an unparseable commit count: {out:?}"))
}

/// Where BugSleuth's own throwaway checkouts live inside a reviewed repository.
///
/// An isolated sweep (Kilo) that was killed rather than dropped leaves one
/// behind, and git reports it as untracked. Judged as the user's uncommitted work it would
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
        // An isolated sweep that was killed leaves a worktree behind, and git
        // calls it untracked. Counting it would refuse every apply — telling
        // the user to commit or stash a directory this tool created — until
        // they deleted it by hand.
        let mixed = vec![
            ".bugsleuth-worktrees/sweep-kilo-1/src/main.rs".to_string(),
            "src/real.rs".to_string(),
        ];
        assert_eq!(theirs(mixed), ["src/real.rs"]);
        // And a repository whose only "changes" are ours reads as clean.
        assert!(theirs(vec![".bugsleuth-worktrees/x".to_string()]).is_empty());
    }

    #[test]
    fn unborn_repository_reports_its_initial_commit_as_a_change() {
        // A repository with a `.git` but no commit is a valid apply target. Its
        // initial commit used to read as no change at all: rev-parse HEAD failed,
        // the baseline collapsed to "no history", and both counts came back
        // empty even though a commit with all the edits now existed.
        let dir = std::env::temp_dir().join(format!("bugsleuth-unborn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !run(&["init", "-q"]) {
            // No usable git here; the pure-logic tests still cover the rest.
            return;
        }
        let _ = run(&["config", "user.email", "t@example.com"]);
        let _ = run(&["config", "user.name", "Tester"]);

        // No commit yet — the unborn baseline apply captures before the model runs.
        let base = Baseline::Unborn;

        let _ = std::fs::write(dir.join("first.txt"), "hello\n");
        let _ = run(&["add", "-A"]);
        if !run(&["commit", "-qm", "initial"]) {
            return;
        }

        assert_eq!(changed_since(&dir, &base).expect("changed"), ["first.txt"]);
        assert_eq!(commits_since(&dir, &base).expect("commits"), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_observation_failure_is_not_clean() {
        // A broken repository must not read as zero commits and no changed
        // files. That is exactly the "absence read as a result" this project
        // exists to stop: it would report a clean apply and, with push on,
        // decide there was nothing to publish — on a repository it could not
        // even open.
        let dir =
            std::env::temp_dir().join(format!("bugsleuth-obsfail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !run(&["init", "-q"]) {
            return; // no usable git here
        }
        let _ = run(&["config", "user.email", "t@example.com"]);
        let _ = run(&["config", "user.name", "Tester"]);
        let _ = std::fs::write(dir.join("a.txt"), "one");
        let _ = run(&["add", "-A"]);
        if !run(&["commit", "-qm", "base"]) {
            return;
        }
        let base = Baseline::Commit(
            git(&dir, &["rev-parse", "HEAD"])
                .expect("head")
                .trim()
                .to_string(),
        );

        // A real change is really seen — so an empty scan cannot masquerade as a
        // clean success and hide the fix below.
        let _ = std::fs::write(dir.join("b.txt"), "two");
        let _ = run(&["add", "-A"]);
        if !run(&["commit", "-qm", "second"]) {
            return;
        }
        assert!(
            changed_since(&dir, &base)
                .expect("changed")
                .contains(&"b.txt".to_string()),
            "a known changed file must be reported"
        );
        assert_eq!(commits_since(&dir, &base).expect("commits"), 1);

        // Now take the repository away underneath them. Both must error rather
        // than answer zero / empty.
        std::fs::remove_dir_all(dir.join(".git")).expect("remove .git");
        assert!(
            changed_since(&dir, &base).is_err(),
            "an unreadable repository must not read as 'no files changed'"
        );
        assert!(
            commits_since(&dir, &base).is_err(),
            "an unreadable repository must not read as 'zero commits'"
        );
        let _ = std::fs::remove_dir_all(&dir);
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
