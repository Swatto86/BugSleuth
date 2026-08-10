//! Cancellation at the irreversible publication boundary.

use super::tests::{git_ok, remote_head, repo_with_a_commit, with_upstream};
use super::*;
use std::time::Duration;

async fn wait_for(path: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{} was never created", path.display()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_publication_stops_push_and_tag() {
    let repo = repo_with_a_commit("cancel");
    let remote = with_upstream(&repo, "cancel-remote");
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);
    let base = remote_head(&remote, &branch);

    git_ok(&repo, &["tag", "-a", "v1.0.0", "-m", "seed"]);
    git_ok(&repo, &["push", "-q", "origin", "v1.0.0"]);
    std::fs::write(repo.join("fix.txt"), "the fix\n").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "the fix"]);

    let hook = remote.join("hooks/pre-receive");
    std::fs::write(
        &hook,
        "#!/bin/sh\nroot=$(cd \"$(dirname \"$0\")/..\" && pwd)\n\
         : > \"$root/push-started\"\n\
         while [ ! -f \"$root/release-hook\" ]; do sleep 1; done\n\
         : > \"$root/hook-survived\"\nexit 1\n",
    )
    .expect("write blocking hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    let cancel = crate::cancel::Cancel::new();
    let push_cancel = cancel.clone();
    let push_repo = repo.clone();
    let push_base = base.clone();
    let mut task = tokio::spawn(async move {
        push(
            &push_repo,
            &Baseline::Commit(push_base),
            1,
            &[],
            &push_cancel,
            Duration::from_secs(10),
        )
        .await
    });
    wait_for(&remote.join("push-started")).await;
    cancel.stop();

    let stopped = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
    let outcome = if let Ok(joined) = stopped {
        joined.expect("push task")
    } else {
        std::fs::write(remote.join("release-hook"), "release\n").expect("release old hook");
        let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        for dir in [&repo, &remote] {
            let _ = std::fs::remove_dir_all(dir);
        }
        panic!("cancellation did not interrupt the blocking git push");
    };

    let PushOutcome::Unknown { error, .. } = &outcome else {
        panic!("an interrupted push was reported with certainty: {outcome:?}");
    };
    assert!(error.contains("may have accepted"), "{error}");
    assert_eq!(remote_head(&remote, &branch), base);

    std::fs::write(remote.join("release-hook"), "release\n").expect("release leaked hook");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        !remote.join("hook-survived").exists(),
        "the receive-hook descendant survived cancellation"
    );

    let tagged = match super::super::to_tag(true, &outcome) {
        Some(remote_name) => {
            super::super::tag::tag(&repo, true, remote_name, &cancel, Duration::from_secs(10)).await
        }
        None => super::super::TagOutcome::NotPushed,
    };
    assert_eq!(tagged, super::super::TagOutcome::NotPushed);
    assert_ne!(git_ok(&repo, &["tag", "--list", "v1.0.1"]), "v1.0.1");
    assert!(!remote_tags_include(&remote, "v1.0.1"));

    for dir in [&repo, &remote] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn remote_tags_include(remote: &Path, tag: &str) -> bool {
    git_ok(remote, &["tag", "--list", tag]) == tag
}
