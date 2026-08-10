//! Exact-ref publication checks, kept separate from the main push tests at the
//! file-size cap.

use super::tests::{git_ok, remote_head, repo_with_a_commit, with_upstream};
use super::*;
use std::time::Duration;

async fn push_now(repo: &Path, base: String) -> PushOutcome {
    push(
        repo,
        &Baseline::Commit(base),
        1,
        &[],
        &crate::cancel::Cancel::new(),
        Duration::from_secs(10),
    )
    .await
}

#[tokio::test]
async fn configured_push_rules_cannot_publish_extra_refs() {
    let repo = repo_with_a_commit("configured-refs");
    let remote = with_upstream(&repo, "configured-refs-remote");
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);
    let base = remote_head(&remote, &branch);

    // An annotated local tag reachable from the branch, but deliberately not
    // published. `push.followTags` must not broaden this apply's publication.
    git_ok(&repo, &["tag", "-a", "private-backup", "-m", "not public"]);

    // A second branch that exists remotely, then advances only locally.
    git_ok(&repo, &["checkout", "-q", "-b", "sibling"]);
    git_ok(&repo, &["push", "-q", "origin", "sibling"]);
    let sibling_before = remote_head(&remote, "sibling");
    std::fs::write(repo.join("sibling.txt"), "private work\n").expect("write sibling");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "private sibling work"]);

    git_ok(&repo, &["checkout", "-q", &branch]);
    std::fs::write(repo.join("fix.txt"), "the fix\n").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);
    let desired = git_ok(&repo, &["rev-parse", "HEAD"]);

    // Both settings broaden a bare `git push`; neither may affect an apply.
    git_ok(&repo, &["config", "push.followTags", "true"]);
    git_ok(
        &repo,
        &[
            "config",
            "--add",
            "remote.origin.push",
            &format!("refs/heads/{branch}:refs/heads/{branch}"),
        ],
    );
    git_ok(
        &repo,
        &[
            "config",
            "--add",
            "remote.origin.push",
            "refs/heads/sibling:refs/heads/sibling",
        ],
    );

    let outcome = push_now(&repo, base).await;
    assert!(
        matches!(outcome, PushOutcome::Pushed { .. }),
        "the intended branch was not published: {outcome:?}"
    );
    assert_eq!(
        remote_head(&remote, &branch),
        desired,
        "the known intended ref did not move, so an empty publication check could pass"
    );

    let sibling_was_published = remote_head(&remote, "sibling") != sibling_before;
    let private_tag_was_published =
        git_ok(&remote, &["tag", "--list", "private-backup"]) == "private-backup";
    assert!(
        !sibling_was_published && !private_tag_was_published,
        "configured rules broadened publication: sibling={sibling_was_published}, tag={private_tag_was_published}"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn push_uses_only_the_frozen_apply_tip() {
    let repo = repo_with_a_commit("frozen-tip");
    let remote = with_upstream(&repo, "frozen-tip-remote");
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);

    std::fs::write(repo.join("fix.txt"), "the frozen fix\n").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the frozen fix"]);
    let frozen = git_ok(&repo, &["rev-parse", "HEAD"]);

    std::fs::write(repo.join("later.txt"), "unrelated later work\n").expect("write later");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "later work"]);
    let live_head = git_ok(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(frozen, live_head, "the test did not advance HEAD");

    super::super::remote::push_ref(
        &repo,
        "origin",
        &frozen,
        &format!("refs/heads/{branch}"),
        &crate::cancel::Cancel::new(),
        Duration::from_secs(10),
    )
    .await
    .expect("push frozen ref");

    assert_eq!(
        remote_head(&remote, &branch),
        frozen,
        "the push followed mutable HEAD instead of its frozen source OID"
    );

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn a_successful_push_without_remote_confirmation_is_unknown() {
    let repo = repo_with_a_commit("unconfirmed-success");
    let remote = with_upstream(&repo, "unconfirmed-success-remote");
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);
    let base = remote_head(&remote, &branch);

    // The receive succeeds, then the server moves the branch back before the
    // client returns. A command exit code alone must not claim publication.
    let hook = remote.join("hooks/post-receive");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ngit update-ref refs/heads/{branch} {base}\n"),
    )
    .expect("write post-receive hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    std::fs::write(repo.join("fix.txt"), "the fix\n").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);

    let outcome = push_now(&repo, base.clone()).await;
    let PushOutcome::Unknown { error, .. } = &outcome else {
        panic!("an unconfirmed success was reported as certain: {outcome:?}");
    };
    assert!(error.contains("reported success"), "{error}");
    assert_eq!(remote_head(&remote, &branch), base);

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}
