//! Tests for the attribution guard, in their own file only because the module
//! plus its tests crossed the hard line cap — the same split `claude/tests.rs`
//! and `worktree/tests.rs` already use.

use super::super::observed::commits_since;
use super::*;
use std::process::Command;

#[test]
fn a_commit_crediting_the_tool_is_spotted_and_an_ordinary_one_is_not() {
    // Subject, NUL, body, RS per commit — exactly the format
    // `attributed_since` asks git for.
    let log = concat!(
        "fix(auth): stop the loop\u{0}Body.\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n\u{1e}",
        "fix(ui): focus the dialog\u{0}Body with no trailer.\n\u{1e}",
        "chore: tidy\u{0}Generated with Codex\n\u{1e}",
        "feat: pairing\u{0}Co-authored-by: Sam <sam@example.com>\n\u{1e}",
        // Names nothing on the list, but signs from a domain that is.
        "docs: notes\u{0}Co-Authored-By: Assistant <noreply@anthropic.com>\n\u{1e}",
    );
    assert_eq!(
        attributed(log),
        ["fix(auth): stop the loop", "chore: tidy", "docs: notes"],
        "a human co-author is not attribution and must not be flagged"
    );
}

#[test]
fn the_format_string_is_the_one_git_actually_produces() {
    // The test above builds the log by hand, which proves the parser and
    // nothing about `git log --format=...`. One separator wrong and the
    // whole warning silently never fires. So: a real repository, real
    // commits, the real format string.
    let dir = std::env::temp_dir().join(format!("bugsleuth-attr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "t@example.com"]);
    run(&["config", "user.name", "Tester"]);
    let _ = std::fs::write(dir.join("a.txt"), "one");
    run(&["add", "-A"]);
    run(&["commit", "-qm", "base"]);
    let base = git(&dir, &["rev-parse", "HEAD"])
        .expect("head")
        .trim()
        .to_string();

    let _ = std::fs::write(dir.join("a.txt"), "two");
    run(&["add", "-A"]);
    run(&[
        "commit",
        "-qm",
        "fix: something real\n\nA body.\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
    ]);
    let _ = std::fs::write(dir.join("a.txt"), "three");
    run(&["add", "-A"]);
    run(&["commit", "-qm", "fix: another\n\nNo trailer here."]);

    assert_eq!(
        attributed_since(&dir, Some(&base)),
        ["fix: something real"],
        "only the credited commit, with its subject intact"
    );
    assert_eq!(commits_since(&dir, Some(&base)), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stripping_removes_the_trailer_and_changes_nothing_else() {
    // The claim this function makes is narrow and total: the trailer lines
    // go, and the code, the author and the dates do not move. Anything less
    // and it is a tool quietly rewriting someone's history.
    let dir = std::env::temp_dir().join(format!("bugsleuth-strip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "t@example.com"]);
    run(&["config", "user.name", "Tester"]);
    let _ = std::fs::write(dir.join("a.txt"), "one");
    run(&["add", "-A"]);
    run(&["commit", "-qm", "base"]);
    let base = git(&dir, &["rev-parse", "HEAD"])
        .expect("head")
        .trim()
        .to_string();

    let _ = std::fs::write(dir.join("a.txt"), "two");
    run(&["add", "-A"]);
    run(&[
        "commit",
        "-qm",
        "fix: the real work\n\nWhy it was wrong.\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
    ]);
    let _ = std::fs::write(dir.join("b.txt"), "three");
    run(&["add", "-A"]);
    run(&["commit", "-qm", "fix: more\n\nNo trailer."]);

    let tree_before = git(&dir, &["rev-parse", "HEAD^{tree}"]).expect("tree");
    let authored_before = git(
        &dir,
        &["log", "--format=%an|%ae|%aI", &format!("{base}..HEAD")],
    )
    .expect("authors");

    let stripped = strip_attribution(&dir, &base).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stripped, ["fix: the real work"], "only the credited commit");

    // The trailer is gone and nothing else is.
    assert!(attributed_since(&dir, Some(&base)).is_empty());
    let log = git(&dir, &["log", "--format=%B", &format!("{base}..HEAD")]).expect("log");
    assert!(!log.contains("Co-Authored-By"), "{log}");
    assert!(log.contains("Why it was wrong."), "the body is kept: {log}");
    assert!(
        log.contains("fix: more"),
        "the untouched commit survives: {log}"
    );
    assert_eq!(commits_since(&dir, Some(&base)), 2, "no commit was lost");
    assert_eq!(
        git(&dir, &["rev-parse", "HEAD^{tree}"]).expect("tree"),
        tree_before,
        "the code itself must be byte-identical"
    );
    assert_eq!(
        git(
            &dir,
            &["log", "--format=%an|%ae|%aI", &format!("{base}..HEAD")]
        )
        .expect("authors"),
        authored_before,
        "the author and the date are the user's, not this process's"
    );
    assert_eq!(
        git(&dir, &["status", "--porcelain"])
            .expect("status")
            .trim(),
        "",
        "the working tree is left clean"
    );

    // And with nothing left to strip it must not touch the history at all.
    // The comment above claims this costs nothing; without this assertion
    // that claim is just prose, and every apply would silently renumber its
    // own commits.
    let head = git(&dir, &["rev-parse", "HEAD"]).expect("head");
    assert!(
        strip_attribution(&dir, &base)
            .expect("second pass")
            .is_empty()
    );
    assert_eq!(
        git(&dir, &["rev-parse", "HEAD"]).expect("head"),
        head,
        "a range with no trailer must be left exactly as it was"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_clean_history_reports_nothing_rather_than_an_empty_subject() {
    // The window keys off `is_empty()`, so one stray blank entry would warn
    // about a commit that does not exist.
    assert!(attributed("").is_empty());
    assert!(attributed("\u{1e}\u{1e}").is_empty());
    assert!(attributed("subject with no separator\u{1e}").is_empty());
}
