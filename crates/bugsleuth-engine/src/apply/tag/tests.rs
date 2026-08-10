//! Tests for tagging a published apply.
//!
//! Same shape as the push tests beside them, and for the same reason: the
//! version arithmetic is checked as pure logic, and then the whole thing is run
//! against a real bare repository, because a tag that does not reach the remote
//! triggers no workflow at all — and that failure is invisible to any test that
//! only inspects the decision.

use super::test_support::{git_ok, published, published_on, remote_tags};
use super::*;
use std::time::Duration;

async fn tag_now(repo: &Path, pushed: bool, remote: &str) -> TagOutcome {
    let commit = git_ok(repo, &["rev-parse", "HEAD"]);
    tag(
        repo,
        pushed,
        remote,
        &commit,
        &crate::cancel::Cancel::new(),
        Duration::from_secs(10),
    )
    .await
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

#[tokio::test]
async fn a_branch_with_no_release_tag_has_no_scheme_to_follow() {
    let (repo, remote, remote_name) = published("unversioned");

    let outcome = tag_now(&repo, true, &remote_name).await;
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

#[tokio::test]
async fn a_tag_on_another_branch_is_never_moved() {
    // The collision that actually reaches this code. The tag list is filtered
    // to `--merged HEAD`, so a release tagged on a branch this one does not
    // contain is invisible when the next version is worked out — and the name
    // it lands on is already taken. Reusing it means moving a tag a published
    // release was built from, onto a different commit.
    let (repo, remote, remote_name) = published("collision");
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

    let outcome = tag_now(&repo, true, &remote_name).await;
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

#[tokio::test]
async fn the_tag_reaches_the_remote_and_points_at_the_fixes() {
    // The wiring test. Everything above proves a decision; this proves the tag
    // actually lands on the remote, on the right commit — which is the only
    // thing that makes CI build a release at all.
    let (repo, remote, remote_name) = published("reaches");
    git_ok(&repo, &["tag", "-a", "v2.4.7", "-m", "the last release"]);
    git_ok(&repo, &["push", "-q", "origin", "v2.4.7"]);

    std::fs::write(repo.join("b.txt"), "the fix\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "fix: the defect"]);
    git_ok(&repo, &["push", "-q"]);
    let head = git_ok(&repo, &["rev-parse", "HEAD"]);

    let outcome = tag_now(&repo, true, &remote_name).await;
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

#[tokio::test]
async fn slash_in_remote_name_does_not_redirect_release_tag() {
    // Git allows a remote named `team/origin`. The tag path used to split the
    // upstream `team/origin/branch` on '/' and derive the non-existent remote
    // `team`, so the release never fired; the exact remote the push used must
    // reach the tag instead.
    let (repo, remote, remote_name) = published_on("slash", "team/origin");
    let base = git_ok(&repo, &["rev-parse", "HEAD"]);
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "released"]);
    git_ok(&repo, &["push", "-q", "team/origin", "v1.0.0"]);
    std::fs::write(repo.join("b.txt"), "the fix\n").expect("write");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "fix: the defect"]);

    // The push must report the exact remote, and the tag must use it verbatim.
    let pushed = super::super::push::push(
        &repo,
        &super::super::Baseline::Commit(base),
        1,
        &[],
        &crate::cancel::Cancel::new(),
        Duration::from_secs(10),
    )
    .await;
    let super::super::PushOutcome::Pushed {
        remote: got, oid, ..
    } = &pushed
    else {
        panic!("the fix was not pushed: {pushed:?}");
    };
    assert_eq!(got, &remote_name, "the push lost the exact remote");

    let outcome = tag(
        &repo,
        true,
        got,
        oid,
        &crate::cancel::Cancel::new(),
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(
        outcome,
        TagOutcome::Tagged {
            tag: "v1.0.1".to_string(),
            remote: "team/origin".to_string(),
        },
        "{outcome:?}"
    );
    assert!(
        remote_tags(&remote).contains(&"v1.0.1".to_string()),
        "the release tag did not reach the slash-named remote"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn only_the_release_tag_is_published() {
    // `git push --tags` sends every local tag, including private backup tags
    // that were deliberately never pushed. One bulk push republished history
    // that had been squashed away on purpose.
    let (repo, remote, remote_name) = published("named-only");
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

    let outcome = tag_now(&repo, true, &remote_name).await;
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

#[tokio::test]
async fn an_unreadable_remote_after_a_failed_tag_push_is_unknown_and_keeps_the_tag() {
    // A push error only means the client got no acknowledgement. If the remote
    // then cannot be read, whether the tag — and the release — landed is
    // genuinely unknown, so the local tag is kept rather than deleted and the
    // outcome is never reported as a definite failure.
    let (repo, remote, remote_name) = published("unreadable");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "released"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);

    // Tag-state preflight succeeds. The receive hook then rejects the tag and
    // makes the repository unreadable before reconciliation can inspect it.
    let hook = remote.join("hooks").join("pre-receive");
    std::fs::write(&hook, "#!/bin/sh\nmv objects objects-unreadable\nexit 1\n")
        .expect("write hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    let outcome = tag_now(&repo, true, &remote_name).await;
    let TagOutcome::Unknown { tag, .. } = &outcome else {
        panic!("an unconfirmable tag push was not reported as unknown: {outcome:?}");
    };
    assert_eq!(tag, "v1.0.1");
    assert_eq!(
        git_ok(&repo, &["tag", "--list", "v1.0.1"]),
        "v1.0.1",
        "the local tag was deleted despite the outcome being unknown"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn a_confirmed_rejection_reports_failure_and_removes_the_local_tag() {
    // When the remote is readable and the tag is confirmed still absent after a
    // failed push, that is a genuine rejection: reported as failed, and the
    // local tag removed so a retry is not blocked by the collision check.
    let (repo, remote, remote_name) = published("denied");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "released"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);

    // A pre-receive hook that rejects every push while the remote stays readable,
    // so reconciliation can confirm the tag never landed.
    let hook = remote.join("hooks").join("pre-receive");
    std::fs::create_dir_all(remote.join("hooks")).expect("hooks dir");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("write hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    let outcome = tag_now(&repo, true, &remote_name).await;
    let TagOutcome::Failed(reason) = &outcome else {
        panic!("a confirmed rejection was not reported as failed: {outcome:?}");
    };
    assert!(!reason.is_empty(), "the failure carried no explanation");
    assert_eq!(
        git_ok(&repo, &["tag", "--list", "v1.0.1"]),
        "",
        "the local tag was left behind after a confirmed rejection"
    );
    assert!(
        !remote_tags(&remote).contains(&"v1.0.1".to_string()),
        "the rejected tag reached the remote after all"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn a_tag_already_on_the_remote_is_refused_even_when_absent_locally() {
    // Another machine may have published the next version. Tagging over it would
    // repoint a release, so a name already on the remote is refused even when
    // this checkout has never seen it.
    let (repo, remote, remote_name) = published("remote-collision");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "released"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);

    // Publish v1.0.1 from here, then forget it locally so the collision is only
    // visible on the remote.
    git_ok(&repo, &["tag", "-a", "v1.0.1", "-m", "elsewhere"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.1"]);
    git_ok(&repo, &["tag", "-d", "v1.0.1"]);

    let outcome = tag_now(&repo, true, &remote_name).await;
    let TagOutcome::Refused(reason) = &outcome else {
        panic!("a remote tag collision was not refused: {outcome:?}");
    };
    assert!(
        reason.contains("tag names and objects do not match origin"),
        "{reason}"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}
