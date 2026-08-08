//! Tests for publishing an apply's commits.
//!
//! The guards are checked as pure logic, and then the whole thing is run
//! against a real bare repository on disk. Unit tests on the decision would
//! prove only the decision: whether the argument list we hand `git push` does
//! what we believe needs a `git push` that really happens, and a remote that
//! can be inspected afterwards.

use super::*;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

// git is not optional here. The engine cannot apply anything without it, so a
// missing git is a broken environment rather than a reason to pass quietly —
// skipping would turn every assertion below into one nobody has ever watched.
fn git_ok(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} could not run: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A throwaway directory, unique per call so tests may run in parallel.
fn scratch(tag: &str) -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("bugsleuth-push-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A working repository with one commit, and its identity configured so
/// committing works on a machine with no global git config (CI).
fn repo_with_a_commit(tag: &str) -> std::path::PathBuf {
    let dir = scratch(tag);
    git_ok(&dir, &["init", "-q"]);
    git_ok(&dir, &["config", "user.email", "t@example.com"]);
    git_ok(&dir, &["config", "user.name", "Tester"]);
    std::fs::write(dir.join("a.txt"), "one\n").expect("write");
    git_ok(&dir, &["add", "-A"]);
    git_ok(&dir, &["commit", "-qm", "first"]);
    dir
}

/// `repo` with a bare remote beside it, already set as the branch's upstream.
fn with_upstream(repo: &Path, tag: &str) -> std::path::PathBuf {
    let remote = scratch(tag);
    git_ok(&remote, &["init", "-q", "--bare"]);
    git_ok(
        repo,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    let branch = git_ok(repo, &["symbolic-ref", "--short", "HEAD"]);
    git_ok(repo, &["push", "-q", "-u", "origin", &branch]);
    remote
}

/// What the remote believes the branch points at.
fn remote_head(remote: &Path, branch: &str) -> String {
    git_ok(remote, &["rev-parse", &format!("refs/heads/{branch}")])
}

#[test]
fn an_apply_that_committed_nothing_has_nothing_to_publish() {
    assert_eq!(blocked(0, &[]), Some(PushOutcome::NothingToPush));
}

#[test]
fn a_commit_that_credits_a_tool_is_never_published() {
    // The whole reason this guard exists. A trailer that could not be stripped
    // is one on a commit that was already published or could not be rewritten,
    // and pushing is what makes it permanent in someone else's history.
    let one = blocked(1, &["fix(auth): stop demanding re-sign-in".to_string()]);
    let Some(PushOutcome::Refused(reason)) = one else {
        panic!("a tool-credited commit was publishable: {one:?}");
    };
    assert!(reason.contains("1 commit"), "{reason}");
    assert!(reason.contains("credits"), "{reason}");
    assert!(reason.contains("Nothing was pushed"), "{reason}");

    let two = blocked(2, &["one".to_string(), "two".to_string()]);
    let Some(PushOutcome::Refused(reason)) = two else {
        panic!("two tool-credited commits were publishable: {two:?}");
    };
    assert!(reason.contains("2 commits"), "{reason}");
    assert!(reason.contains("credit "), "{reason}");
}

#[test]
fn a_clean_apply_with_commits_is_publishable() {
    // Without this the three assertions above would all pass on a function
    // that refused everything, which is the failure mode a guard like this
    // actually has.
    assert_eq!(blocked(3, &[]), None);
}

#[test]
fn a_detached_head_is_refused_because_there_is_no_branch_to_push() {
    let repo = repo_with_a_commit("detached");
    let head = git_ok(&repo, &["rev-parse", "HEAD"]);
    git_ok(&repo, &["checkout", "-q", "--detach", &head]);

    let outcome = push(&repo, &Baseline::Commit(head), 1, &[]);
    let PushOutcome::Refused(reason) = &outcome else {
        panic!("a detached HEAD was pushed: {outcome:?}");
    };
    assert!(reason.contains("detached"), "{reason}");
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn a_branch_with_no_upstream_is_left_alone() {
    // No remote is configured at all, so there is nowhere it has been agreed to
    // publish. Guessing "origin" here is how a private branch reaches a shared
    // remote for the first time without anyone asking for it.
    let repo = repo_with_a_commit("no-upstream");

    let outcome = push(
        &repo,
        &Baseline::Commit(git_ok(&repo, &["rev-parse", "HEAD"])),
        1,
        &[],
    );
    let PushOutcome::Refused(reason) = &outcome else {
        panic!("a branch with no upstream was pushed: {outcome:?}");
    };
    assert!(reason.contains("no upstream"), "{reason}");
    assert!(reason.contains("git push -u"), "{reason}");
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn the_commits_actually_reach_the_remote() {
    // The wiring test. Everything above proves a decision; this proves the
    // command we build out of that decision moves commits to a remote that can
    // be inspected afterwards.
    let repo = repo_with_a_commit("reaches");
    let remote = with_upstream(&repo, "reaches-remote");
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);
    let before = remote_head(&remote, &branch);

    std::fs::write(repo.join("b.txt"), "two\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);
    let local = git_ok(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(local, before, "the test did not create a new commit");

    let outcome = push(&repo, &Baseline::Commit(before), 1, &[]);
    assert_eq!(
        outcome,
        PushOutcome::Pushed {
            branch: branch.clone(),
            upstream: format!("origin/{branch}"),
        },
        "{outcome:?}"
    );
    assert_eq!(
        remote_head(&remote, &branch),
        local,
        "the push reported success but the remote did not move"
    );

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&remote);
}

#[test]
fn commits_that_predate_the_apply_are_not_published() {
    let repo = repo_with_a_commit("predates");
    let remote = with_upstream(&repo, "predates-remote");
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);
    let remote_before = remote_head(&remote, &branch);

    std::fs::write(repo.join("unrelated.txt"), "not part of the apply\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "unrelated unpublished work"]);
    let base = git_ok(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("fix.txt"), "the apply\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);

    let outcome = push(&repo, &Baseline::Commit(base), 1, &[]);
    assert!(
        matches!(outcome, PushOutcome::Refused(_)),
        "the unrelated commit was publishable: {outcome:?}"
    );
    assert_eq!(remote_head(&remote, &branch), remote_before);

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&remote);
}

#[test]
fn only_the_branch_that_was_applied_to_is_published() {
    // `push.default = matching` is git's old default and is still set in plenty
    // of long-lived configs. Under it a bare `git push` publishes *every* local
    // branch whose name matches one on the remote — so fixing a defect on one
    // branch would push whatever else happened to be lying around, which is the
    // worst possible surprise from a checkbox labelled "push the commits".
    let repo = repo_with_a_commit("matching");
    let remote = with_upstream(&repo, "matching-remote");
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);
    git_ok(&repo, &["config", "push.default", "matching"]);

    // A second branch that exists on both sides, then moves on only locally.
    git_ok(&repo, &["checkout", "-q", "-b", "sibling"]);
    git_ok(&repo, &["push", "-q", "origin", "sibling"]);
    let sibling_before = remote_head(&remote, "sibling");
    std::fs::write(repo.join("sibling.txt"), "not yours to publish\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "unrelated work in progress"]);
    assert_ne!(
        git_ok(&repo, &["rev-parse", "HEAD"]),
        sibling_before,
        "the sibling branch did not actually move"
    );

    // Back to the branch the apply happened on, and commit there too.
    git_ok(&repo, &["checkout", "-q", &branch]);
    std::fs::write(repo.join("b.txt"), "the fix\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);
    let ours = git_ok(&repo, &["rev-parse", "HEAD"]);

    let base = git_ok(&repo, &["rev-parse", "HEAD~1"]);
    let outcome = push(&repo, &Baseline::Commit(base), 1, &[]);
    assert!(
        matches!(outcome, PushOutcome::Pushed { .. }),
        "the push did not succeed: {outcome:?}"
    );
    assert_eq!(
        remote_head(&remote, &branch),
        ours,
        "our branch was not published"
    );
    assert_eq!(
        remote_head(&remote, "sibling"),
        sibling_before,
        "an unrelated branch was published too"
    );

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&remote);
}

#[test]
fn a_rejected_push_is_reported_rather_than_forced() {
    // The remote moves on underneath us, so a plain push is refused as a
    // non-fast-forward. The tool must report that and stop: recovering means a
    // rebase or a force, and a force here would discard whatever the other
    // commit was.
    let repo = repo_with_a_commit("rejected");
    let remote = with_upstream(&repo, "rejected-remote");
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);

    // A second clone stands in for whoever else pushed.
    let other = scratch("rejected-other");
    let _ = std::fs::remove_dir_all(&other);
    git_ok(
        &std::env::temp_dir(),
        &[
            "clone",
            "-q",
            &remote.to_string_lossy(),
            &other.to_string_lossy(),
        ],
    );
    git_ok(&other, &["config", "user.email", "o@example.com"]);
    git_ok(&other, &["config", "user.name", "Other"]);
    std::fs::write(other.join("theirs.txt"), "theirs\n").expect("write");
    git_ok(&other, &["add", "-A"]);
    git_ok(&other, &["commit", "-qm", "someone else's work"]);
    git_ok(&other, &["push", "-q"]);
    let theirs = remote_head(&remote, &branch);

    // Now our own commit, on the stale branch.
    std::fs::write(repo.join("b.txt"), "two\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);

    let base = git_ok(&repo, &["rev-parse", "HEAD~1"]);
    let outcome = push(&repo, &Baseline::Commit(base), 1, &[]);
    let PushOutcome::Refused(reason) = &outcome else {
        panic!("a branch whose upstream moved was not refused: {outcome:?}");
    };
    assert!(!reason.is_empty(), "the failure carried no explanation");
    assert_eq!(
        remote_head(&remote, &branch),
        theirs,
        "the remote moved, so the rejected push was forced through after all"
    );

    for dir in [&repo, &remote, &other] {
        let _ = std::fs::remove_dir_all(dir);
    }
}
