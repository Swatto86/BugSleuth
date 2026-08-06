//! Tests for tagging a published apply.
//!
//! Same shape as the push tests beside them, and for the same reason: the
//! version arithmetic is checked as pure logic, and then the whole thing is run
//! against a real bare repository, because a tag that does not reach the remote
//! triggers no workflow at all — and that failure is invisible to any test that
//! only inspects the decision.

use super::*;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

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

fn scratch(tag: &str) -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("bugsleuth-tag-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A repository with one commit and a bare remote it already pushes to — the
/// state this code only ever runs in, since it runs after a successful push.
fn published(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, String) {
    let repo = scratch(tag);
    git_ok(&repo, &["init", "-q"]);
    git_ok(&repo, &["config", "user.email", "t@example.com"]);
    git_ok(&repo, &["config", "user.name", "Tester"]);
    std::fs::write(repo.join("a.txt"), "one\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "first"]);

    let remote = scratch(&format!("{tag}-remote"));
    git_ok(&remote, &["init", "-q", "--bare"]);
    git_ok(
        &repo,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);
    git_ok(&repo, &["push", "-q", "-u", "origin", &branch]);
    (repo, remote, format!("origin/{branch}"))
}

/// Every tag the remote actually has.
fn remote_tags(remote: &Path) -> Vec<String> {
    git_ok(remote, &["tag", "--list"])
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn the_patch_number_is_the_one_that_moves() {
    assert_eq!(next_patch("v1.2.3").as_deref(), Some("v1.2.4"));
    assert_eq!(next_patch("v0.0.0").as_deref(), Some("v0.0.1"));
    // Version order, not string order: the tag list is sorted by `-v:refname`
    // so this is the input a repository past its ninth patch actually gives.
    assert_eq!(next_patch("v1.9.9").as_deref(), Some("v1.9.10"));
}

#[test]
fn a_version_scheme_this_does_not_understand_is_declined_rather_than_guessed() {
    // Each of these is a real tagging convention. Publishing `v1.2.4` into any
    // of them is worse than declining, because the wrong tag has already
    // started someone's release by the time anyone reads the report.
    for odd in [
        "2024.11",       // date-based
        "v1.2",          // two-part
        "v1.2.3-rc1",    // pre-release suffix
        "v1.2.3+build7", // build metadata
        "release-4",     // no version at all
        "1.2.3",         // no v prefix, so not what `tag --list v*` returns
        "v1.2.3.4",      // four parts
        "vx.y.z",        // not numbers
    ] {
        assert_eq!(next_patch(odd), None, "{odd} was treated as a version");
    }
}

#[test]
fn nothing_is_tagged_when_the_commits_were_never_published() {
    // The ordering rule. A tag pushed ahead of its commits starts a build of a
    // ref the runner cannot fetch.
    assert_eq!(blocked(false), Some(TagOutcome::NotPushed));
    // And the guard must not refuse everything, which is the failure mode it
    // would otherwise have and which every assertion above would still pass on.
    assert_eq!(blocked(true), None);
}

#[test]
fn a_branch_with_no_release_tag_has_no_scheme_to_follow() {
    let (repo, remote, upstream) = published("unversioned");

    let outcome = tag(&repo, true, &upstream);
    let TagOutcome::Refused(reason) = &outcome else {
        panic!("a repository with no tags invented a version: {outcome:?}");
    };
    assert!(reason.contains("no `v*` release tag"), "{reason}");
    assert!(
        remote_tags(&remote).is_empty(),
        "a tag reached the remote anyway"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_tag_on_another_branch_is_never_moved() {
    // The collision that actually reaches this code. The tag list is filtered
    // to `--merged HEAD`, so a release tagged on a branch this one does not
    // contain is invisible when the next version is worked out — and the name
    // it lands on is already taken. Reusing it means moving a tag a published
    // release was built from, onto a different commit.
    let (repo, remote, upstream) = published("collision");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "one"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);

    // v1.0.1 released from a branch that was never merged back.
    git_ok(&repo, &["checkout", "-q", "-b", "hotfix"]);
    std::fs::write(repo.join("hotfix.txt"), "released elsewhere\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the hotfix"]);
    git_ok(&repo, &["tag", "-a", "v1.0.1", "-m", "two"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.1"]);
    let before = git_ok(&repo, &["rev-parse", "v1.0.1^{}"]);

    // Back to the branch the apply happened on, whose latest merged tag is
    // still v1.0.0 — so the next patch it works out is the taken name.
    git_ok(&repo, &["checkout", "-q", &branch]);
    std::fs::write(repo.join("b.txt"), "the fix\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);
    git_ok(&repo, &["push", "-q"]);

    let outcome = tag(&repo, true, &upstream);
    let TagOutcome::Refused(reason) = &outcome else {
        panic!("an existing tag was reused: {outcome:?}");
    };
    assert!(reason.contains("v1.0.1 already exists"), "{reason}");
    assert_eq!(
        git_ok(&repo, &["rev-parse", "v1.0.1^{}"]),
        before,
        "the existing tag was moved"
    );
    assert_eq!(
        git_ok(&remote, &["rev-parse", "v1.0.1^{}"]),
        before,
        "the published tag was moved on the remote"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn the_tag_reaches_the_remote_and_points_at_the_fixes() {
    // The wiring test. Everything above proves a decision; this proves the tag
    // actually lands on the remote, on the right commit — which is the only
    // thing that makes CI build a release at all.
    let (repo, remote, upstream) = published("reaches");
    git_ok(&repo, &["tag", "-a", "v2.4.7", "-m", "the last release"]);
    git_ok(&repo, &["push", "-q", "origin", "v2.4.7"]);

    std::fs::write(repo.join("b.txt"), "the fix\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "fix: the defect"]);
    git_ok(&repo, &["push", "-q"]);
    let head = git_ok(&repo, &["rev-parse", "HEAD"]);

    let outcome = tag(&repo, true, &upstream);
    assert_eq!(
        outcome,
        TagOutcome::Tagged {
            tag: "v2.4.8".to_string(),
            remote: "origin".to_string(),
        },
        "{outcome:?}"
    );

    let published_tags = remote_tags(&remote);
    assert!(
        published_tags.contains(&"v2.4.8".to_string()),
        "the tag never reached the remote: {published_tags:?}"
    );
    assert_eq!(
        git_ok(&remote, &["rev-parse", "v2.4.8^{}"]),
        head,
        "the published tag does not point at the fixes"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn only_the_release_tag_is_published() {
    // `git push --tags` sends every local tag, including private backup tags
    // that were deliberately never pushed. One bulk push republished history
    // that had been squashed away on purpose.
    let (repo, remote, upstream) = published("named-only");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "released"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);
    git_ok(
        &repo,
        &["tag", "-a", "pre-squash-backup", "-m", "not yours"],
    );

    std::fs::write(repo.join("b.txt"), "the fix\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "fix: the defect"]);
    git_ok(&repo, &["push", "-q"]);

    let outcome = tag(&repo, true, &upstream);
    assert!(
        matches!(outcome, TagOutcome::Tagged { .. }),
        "the tag was not published: {outcome:?}"
    );

    let published_tags = remote_tags(&remote);
    assert!(
        published_tags.contains(&"v1.0.1".to_string()),
        "the release tag is missing, so this test proved nothing: {published_tags:?}"
    );
    assert!(
        !published_tags.contains(&"pre-squash-backup".to_string()),
        "a private backup tag was published: {published_tags:?}"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_rejected_tag_push_leaves_nothing_behind_locally() {
    // If the local tag survived a failed push, every retry would hit the
    // "already exists" refusal and could never succeed — a dead end that reads
    // as the feature being broken.
    let (repo, remote, upstream) = published("rejected");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "released"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);

    // The remote goes away, so pushing the tag fails.
    std::fs::remove_dir_all(&remote).expect("remove the remote");

    let outcome = tag(&repo, true, &upstream);
    let TagOutcome::Failed(reason) = &outcome else {
        panic!("a failed tag push was not reported: {outcome:?}");
    };
    assert!(!reason.is_empty(), "the failure carried no explanation");
    assert_eq!(
        git_ok(&repo, &["tag", "--list", "v1.0.1"]),
        "",
        "the local tag was left behind after a failed push"
    );

    let _ = std::fs::remove_dir_all(&repo);
}
