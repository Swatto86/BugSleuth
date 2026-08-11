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
        Baseline::Commit(base) => nul_paths(&git(repo, &["diff", "--name-only", "-z", base])?),
        // Started from no commit at all. If the model committed, everything now
        // in HEAD is new against the empty tree the repository began at; if it
        // did not, HEAD is still unborn and only the working-tree scans apply.
        // `range_since` tells those two apart from a broken repository.
        Baseline::Unborn => match range_since(repo, base)? {
            Some(_) => {
                let mut committed =
                    nul_paths(&git(repo, &["ls-tree", "-r", "--name-only", "-z", "HEAD"])?);
                committed.extend(dirty_files(&git(repo, &["status", "--porcelain", "-z"])?));
                committed
            }
            None => dirty_files(&git(repo, &["status", "--porcelain", "-z"])?),
        },
    };
    files.extend(nul_paths(&git(
        repo,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?));
    files.sort();
    files.dedup();
    Ok(theirs(repo, files))
}

/// The `base..HEAD` revision range, or `None` when an unborn baseline still has
/// no commits — the model committed nothing, which is a genuine empty result
/// rather than a failure to inspect.
///
/// The two are told apart the way [`super::baseline`] tells them apart, through
/// the same helper — this asked `rev-list --all --count` separately, which is a
/// question about the whole repository and gave a different answer from the
/// baseline on an orphan branch. Any other Git failure is propagated, never read
/// as "nothing here": a damaged `.git` must not masquerade as an empty history.
pub(super) fn range_since(repo: &Path, base: &Baseline) -> anyhow::Result<Option<String>> {
    match base {
        Baseline::Commit(base) => Ok(Some(format!("{base}..HEAD"))),
        Baseline::Unborn => Ok(super::preflight::current_head(repo)?.map(|_| "HEAD".to_string())),
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

mod ownership;
pub(super) use ownership::theirs;

/// The paths in `git status --porcelain -z` output.
///
/// NUL-delimited machine format, not the human one: no C-quoting of unusual
/// names, no backslash escaping, and no ` -> ` rename arrow to be mistaken for
/// part of a filename. The status letters occupy the first two columns and a
/// space follows, so the path starts at byte three. A rename or copy is two
/// records — the new path here, then the old path in the very next record — and
/// the new path is the one that exists, so the old record is consumed and
/// dropped. git already writes repository-relative paths with `/`.
pub(super) fn dirty_files(porcelain: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut records = porcelain.split('\0');
    while let Some(record) = records.next() {
        // A record too short to hold "XY path" — the trailing empty after the
        // final NUL, or a clean tree — has no path.
        let Some(path) = record.get(3..) else {
            continue;
        };
        // A rename or copy carries its source path in the next record; skip it.
        let status = &record[..2];
        if status.contains('R') || status.contains('C') {
            records.next();
        }
        if !path.is_empty() {
            out.push(path.to_string());
        }
    }
    out
}

/// The paths in a NUL-delimited git listing (`diff -z`, `ls-tree -z`,
/// `ls-files -z`).
///
/// No trimming, quote stripping or backslash handling: `-z` output is the raw
/// path, so a name with a leading space or a literal backslash survives, and git
/// writes repository-relative paths with `/` already.
pub(super) fn nul_paths(out: &str) -> Vec<String> {
    out.split_terminator('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
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
    // A reviewed repository is untrusted: its `.git/config` can point
    // `core.fsmonitor` at an executable and set a hooks path the same way.
    // Disable both ahead of the subcommand, matching revision.rs / gaps.rs /
    // worktree.rs, so apply cannot execute repository-controlled code.
    let output = bugsleuth_verify::hide_console_window(&mut command)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
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
mod fsmonitor_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dirty_tree_is_read_out_of_git_status_whatever_the_status_letters_are() {
        // `-z` records: NUL-terminated, and a rename is the new path then the
        // old path as two separate records.
        let porcelain = " M src/main.rs\0?? new.txt\0A  added.rs\0R  renamed.rs\0old.rs\0";
        let dirty = dirty_files(porcelain);
        assert_eq!(dirty, ["src/main.rs", "new.txt", "added.rs", "renamed.rs"]);
    }

    #[test]
    fn git_paths_preserve_unicode_and_arrow() {
        // The bug the `-z` format fixes: non-`-z` porcelain C-quotes `café.rs`
        // as `"caf\303\251.rs"` and the old parser's backslash-to-slash turned
        // it into `caf/303/251.rs`; a name containing ` -> ` was truncated as if
        // it were rename syntax; a literal backslash became a slash. `-z` output
        // is raw, so each survives intact.
        let porcelain = concat!(
            " M src/café.rs\0",    // raw UTF-8, not C-quoted
            "?? a -> b.rs\0",      // the arrow is part of the name
            "?? weird\\name.rs\0", // a literal backslash is not a separator
            "R  dst.rs\0src.rs\0", // rename: new path, then old path
        );
        assert_eq!(
            dirty_files(porcelain),
            ["src/café.rs", "a -> b.rs", "weird\\name.rs", "dst.rs"]
        );

        // The plain-listing parser keeps the same names untrimmed and unescaped.
        assert_eq!(
            nul_paths("src/café.rs\0a -> b.rs\0weird\\name.rs\0"),
            ["src/café.rs", "a -> b.rs", "weird\\name.rs"]
        );

        // And through real git with `-z`: a known ordinary file proves the scan
        // ran, and the Unicode name proves the flag is actually passed.
        let dir = std::env::temp_dir().join(format!("bugsleuth-gitpaths-{}", std::process::id()));
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
        if run(&["init", "-q"]) {
            let _ = std::fs::write(dir.join("plain.rs"), "x");
            let _ = std::fs::write(dir.join("café.rs"), "y");
            let changed = changed_since(&dir, &Baseline::Unborn).expect("changed");
            assert!(
                changed.contains(&"plain.rs".to_string()),
                "the ordinary file was lost, so an empty scan could pass: {changed:?}"
            );
            assert!(
                changed.contains(&"café.rs".to_string()),
                "the unicode name did not survive real `-z` output: {changed:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clean_tree_reads_as_clean_rather_than_as_one_empty_path() {
        // The guard is `is_empty()`, so a blank record surviving the parse would
        // refuse to apply anything on a perfectly clean repository.
        assert!(dirty_files("").is_empty());
        assert!(dirty_files("\0\0").is_empty());
        // A status record with the letters and nothing after them. The path is
        // then the empty string, and one of those in the list is enough to
        // refuse to apply anything on a repository that is perfectly clean.
        assert!(dirty_files("?? \0").is_empty());
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
        let dir = std::env::temp_dir().join(format!("bugsleuth-obsfail-{}", std::process::id()));
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
