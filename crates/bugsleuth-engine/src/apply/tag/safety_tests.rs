//! Release-version safety checks against real local and remote tag state.

use super::test_support::{git_ok, published, remote_tags};
use super::*;
use std::time::Duration;

async fn tag_now(repo: &Path, remote: &str) -> TagOutcome {
    let commit = git_ok(repo, &["rev-parse", "HEAD"]);
    tag_at(repo, remote, &commit).await
}

async fn tag_at(repo: &Path, remote: &str, commit: &str) -> TagOutcome {
    tag(
        repo,
        true,
        remote,
        commit,
        &crate::cancel::Cancel::new(),
        Duration::from_secs(10),
    )
    .await
}

#[tokio::test]
async fn an_unreadable_remote_tag_view_is_refused_before_selecting_a_version() {
    let (repo, remote, remote_name) = published("unreadable-version-tags");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "released"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);
    std::fs::remove_dir_all(&remote).expect("remove remote");

    let outcome = tag_now(&repo, &remote_name).await;
    let TagOutcome::Refused(reason) = &outcome else {
        panic!("an unreadable remote tag view was treated as safe: {outcome:?}");
    };
    assert!(reason.contains("could not be read"), "{reason}");
    assert_eq!(git_ok(&repo, &["tag", "--list", "v1.0.1"]), "");

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn stale_local_tags_cannot_publish_an_older_release() {
    let (repo, remote, remote_name) = published("stale-version-tags");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "first release"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);
    git_ok(&repo, &["tag", "-a", "v1.0.2", "-m", "newer release"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.2"]);
    git_ok(&repo, &["tag", "-d", "v1.0.2"]);

    assert!(remote_tags(&remote).contains(&"v1.0.2".to_string()));
    assert_eq!(git_ok(&repo, &["tag", "--list", "v1.0.2"]), "");

    std::fs::write(repo.join("fix.txt"), "the fix\n").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);
    git_ok(&repo, &["push", "-q"]);

    let outcome = tag_now(&repo, &remote_name).await;
    let TagOutcome::Refused(reason) = &outcome else {
        panic!("stale local tags selected an older release: {outcome:?}");
    };
    assert!(
        reason.contains("tag names and objects do not match origin"),
        "{reason}"
    );
    assert_eq!(git_ok(&repo, &["tag", "--list", "v1.0.1"]), "");
    assert!(!remote_tags(&remote).contains(&"v1.0.1".to_string()));

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn same_name_tags_with_different_objects_are_refused() {
    let (repo, remote, remote_name) = published("divergent-version-tags");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "remote release"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);
    let remote_tag = git_ok(&remote, &["rev-parse", "refs/tags/v1.0.0"]);
    git_ok(&repo, &["tag", "-d", "v1.0.0"]);

    std::fs::write(repo.join("fix.txt"), "the fix\n").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);
    git_ok(&repo, &["push", "-q"]);
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "local rewrite"]);
    let local_tag = git_ok(&repo, &["rev-parse", "refs/tags/v1.0.0"]);
    assert_ne!(local_tag, remote_tag, "the test tags did not diverge");

    let outcome = tag_now(&repo, &remote_name).await;
    let TagOutcome::Refused(reason) = &outcome else {
        panic!("divergent tag objects were treated as one release: {outcome:?}");
    };
    assert!(
        reason.contains("tag names and objects do not match origin"),
        "{reason}"
    );
    assert_eq!(
        git_ok(&remote, &["rev-parse", "refs/tags/v1.0.0"]),
        remote_tag
    );
    assert_eq!(git_ok(&repo, &["tag", "--list", "v1.0.1"]), "");
    assert!(!remote_tags(&remote).contains(&"v1.0.1".to_string()));

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn the_confirmed_pushed_commit_controls_version_history() {
    let (repo, remote, remote_name) = published("confirmed-version-history");
    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "released"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);

    std::fs::write(repo.join("fix.txt"), "the pushed fix\n").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the pushed fix"]);
    git_ok(&repo, &["push", "-q"]);
    let pushed = git_ok(&repo, &["rev-parse", "HEAD"]);

    std::fs::write(repo.join("later.txt"), "later work\n").expect("write later");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "later work"]);
    git_ok(&repo, &["tag", "-a", "v9.0.0", "-m", "later history"]);
    git_ok(&repo, &["push", "-q", "origin", "v9.0.0"]);

    let outcome = tag_at(&repo, &remote_name, &pushed).await;
    assert_eq!(
        outcome,
        TagOutcome::Tagged {
            tag: "v1.0.1".to_string(),
            remote: "origin".to_string(),
        },
        "version selection followed mutable HEAD: {outcome:?}"
    );
    assert!(remote_tags(&remote).contains(&"v1.0.1".to_string()));
    assert!(!remote_tags(&remote).contains(&"v9.0.1".to_string()));

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}
