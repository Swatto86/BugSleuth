//! Tests for collecting leaked throwaway branches.
//!
//! The dangerous direction here is not "a leaked branch survives" — that is the
//! old behaviour and merely untidy. It is deleting a branch a live worktree is
//! standing on, which takes a running isolated sweep down with it. So the
//! parsing that decides "live" is tested against the listings git really
//! produces, including the ones that carry no branch at all.

use super::super::{Worktree, git, long_path};
use super::*;
use std::path::Path;

/// The listing `git worktree list --porcelain` produces: stanzas separated by
/// blank lines, each naming a path, a HEAD, and either a branch or `detached`.
const LISTING: &str = "\
worktree C:/repo
HEAD 1111111111111111111111111111111111111111
branch refs/heads/master

worktree C:/repo/.bugsleuth-worktrees/correctness-100-0
HEAD 2222222222222222222222222222222222222222
branch refs/heads/bugsleuth/correctness-100-0

worktree C:/repo/.bugsleuth-worktrees/security-100-1
HEAD 3333333333333333333333333333333333333333
detached
";

#[test]
fn every_checked_out_branch_is_read_out_of_the_listing() {
    let live = checked_out(LISTING);

    // Asserted by name, not by count. A parser that silently found nothing
    // would satisfy "no unexpected entries" perfectly, and the sweep built on
    // it would then delete every branch in the repository.
    assert!(
        live.contains("bugsleuth/correctness-100-0"),
        "a live throwaway branch was not seen as live: {live:?}"
    );
    assert!(
        live.contains("master"),
        "the main worktree's branch was not seen as live: {live:?}"
    );

    // A detached worktree contributes no branch, so nothing invented one.
    assert_eq!(live.len(), 2, "{live:?}");
}

#[test]
fn a_listing_with_no_worktrees_yields_no_live_branches() {
    // The empty case has to be genuinely empty rather than accidentally
    // containing a stray "" that would then match a branch name.
    assert!(checked_out("").is_empty());
    assert!(checked_out("worktree C:/repo\nHEAD 1111\ndetached\n").is_empty());
}

#[test]
fn only_a_real_branch_line_counts_as_one() {
    // `branch` lines carry a full ref. Anything else in the listing — a path
    // that happens to contain the word, a bare `branch` with no ref — must not
    // be read as a checked-out branch, or a live worktree could be missed and
    // its branch deleted underneath it.
    let odd = "\
worktree C:/repo/branch refs/heads/decoy
HEAD 4444444444444444444444444444444444444444
branch refs/heads/real
";
    let live = checked_out(odd);
    assert!(
        live.contains("real"),
        "the genuine branch was missed: {live:?}"
    );
    assert_eq!(live.len(), 1, "a decoy was read as a branch: {live:?}");

    // A ref outside refs/heads is not a branch name this sweep compares against.
    assert!(checked_out("branch refs/tags/v1.0.0\n").is_empty());
}

#[test]
fn only_branches_this_tool_generated_are_ever_swept() {
    // The real generated shape: prefix, label, pid, counter. Labels contain
    // hyphens, which is why the split is from the right.
    assert!(ours("bugsleuth/correctness-35604-0"));
    assert!(ours("bugsleuth/prefix-check-35604-0"));
    assert!(ours("bugsleuth/sweep-kilo-25584-12"));

    // Nothing reserves this prefix in someone else's repository. A branch a
    // person made and is working on must survive — deleting it is data loss,
    // and this tool operates on repositories it does not own.
    assert!(!ours("bugsleuth/faster-sweeps"));
    assert!(!ours("bugsleuth/fix-the-thing-v2"));
    assert!(!ours("bugsleuth/"));
    assert!(!ours("bugsleuth/-1-0"), "an empty label is not a real name");

    // And nothing outside the prefix is ever a candidate.
    assert!(!ours("master"));
    assert!(!ours("feature/bugsleuth/correctness-1-0"));
}

#[test]
fn the_sweep_matches_the_shape_of_a_real_throwaway_branch() {
    // The whole sweep keys off this prefix, and a prefix that drifted from the
    // one `Worktree::create` builds would match nothing while looking exactly
    // like a sweep with nothing to collect. `create` formats its name from this
    // same constant, so the compiler holds that end; this pins the value, since
    // changing it would orphan every branch made by an older build.
    assert_eq!(PREFIX, "bugsleuth/");

    // And a real name — label, pid, counter — is matched by it.
    assert!(format!("{PREFIX}correctness-35604-0").starts_with(PREFIX));
}

/// A run that is killed rather than dropped must not leak its branch.
///
/// The defect this reproduces: cleanup lived only in `Drop`, and the
/// orphan sweep removed leftover *directories* but never the refs beside
/// them. So every killed run left a `bugsleuth/<label>-<pid>-<n>` branch
/// behind for good — the unique naming means nothing ever reuses one — and
/// they accumulated silently in the user's own repository. Two were found
/// in this repository, seventeen hours after the runs that made them.
#[test]
fn a_branch_left_by_a_killed_run_is_swept_up_by_the_next_one() {
    let dir = std::env::temp_dir()
        .join("bugsleuth-worktree-leak")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(long_path(&dir));
    if std::fs::create_dir_all(&dir).is_err() || git(&dir, &["init", "-q"]).is_err() {
        return;
    }
    let _ = git(&dir, &["config", "user.email", "t@example.invalid"]);
    let _ = git(&dir, &["config", "user.name", "test"]);
    let _ = std::fs::write(dir.join("a.txt"), "hello\n");
    let _ = git(&dir, &["add", "-A"]);
    if git(&dir, &["commit", "-qm", "base"]).is_err() {
        return;
    }

    // What a killed run leaves: a `bugsleuth/*` branch with no worktree on it.
    //
    // The process id in the name is deliberately NOT this process's. A run only
    // ever deletes its own name before reusing it (`branch -D` in `create`), so
    // a leaked branch carrying the live pid would be collected by that line
    // instead of by the sweep — and this test would pass with the sweep removed
    // entirely. It did, before this comment existed. The foreign pid is what
    // makes the sweep the only thing that can collect it.
    let leaked = format!("{PREFIX}killed-run-{}-0", std::process::id() ^ 0x5eed);
    if git(&dir, &["branch", &leaked, "HEAD"]).is_err() {
        return;
    }

    // Asserted present first: without this the check below would pass just
    // as happily against a branch that was never created, which is the way
    // a test like this goes vacuous.
    assert!(
        branch_exists(&dir, &leaked),
        "the test did not manage to leak a branch, so it proves nothing"
    );

    // A worktree that is still in use — standing in for a concurrent BugSleuth
    // against the same repository. Created BEFORE the sweep runs, because that
    // is the only ordering in which the sweep can see it and get it wrong: the
    // sweeping run's own branch does not exist yet when it sweeps.
    let live = Worktree::create(&dir, "HEAD", "still-running").expect("a live worktree");
    let live_branch = live.branch.clone();
    assert!(
        branch_exists(&dir, &live_branch),
        "the live worktree's branch was never created, so this proves nothing"
    );

    // The next run sweeps, with both a leaked branch and a live one present.
    let next = Worktree::create(&dir, "HEAD", "later-run").expect("a later worktree");
    assert!(
        !branch_exists(&dir, &leaked),
        "{leaked} survived a later run, so killed runs still leak branches"
    );

    // A concurrent run's branch survives — the worse of the two failure modes,
    // since it takes a running isolated sweep down rather than merely leaving
    // litter.
    //
    // What this does not prove: it holds whether the protection comes from the
    // liveness filter or from git's refusal to delete a checked-out branch, and
    // it stays green with the filter removed because git catches it. The filter
    // itself is covered directly by the `checked_out` tests in orphans/tests.rs.
    assert!(
        branch_exists(&dir, &live_branch),
        "the sweep deleted {live_branch}, which a live worktree is standing on"
    );

    drop(next);
    drop(live);
    let _ = std::fs::remove_dir_all(long_path(&dir));
}

/// Whether a branch is still in the repository.
fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
}
