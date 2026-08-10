//! Repository safety guards around attribution rewriting.

use super::*;
use std::process::Command;
use std::time::Duration;

fn git_ok(dir: &Path, args: &[&str]) -> String {
    git(dir, args).unwrap_or_else(|error| panic!("git {args:?}: {error}"))
}

fn attributed_repo(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, String, String) {
    let root = std::env::temp_dir().join(format!("bugsleuth-attr-{tag}-{}", std::process::id()));
    let repo = root.join("repo");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&repo).expect("create repository");
    git_ok(&repo, &["init", "-q"]);
    git_ok(&repo, &["config", "user.email", "t@example.com"]);
    git_ok(&repo, &["config", "user.name", "Tester"]);
    std::fs::write(repo.join("a.txt"), "base").expect("write base");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "base"]);
    let base = git_ok(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    std::fs::write(repo.join("a.txt"), "credited fix").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(
        &repo,
        &[
            "commit",
            "-qm",
            "fix\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
        ],
    );
    let credited = git_ok(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    (root, repo, base, credited)
}

#[tokio::test]
async fn attribution_strip_does_not_flatten_merge_commits() {
    let root = std::env::temp_dir().join(format!("bugsleuth-attr-merge-{}", std::process::id()));
    let repo = root.join("repo");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&repo).expect("create repository");
    git_ok(&repo, &["init", "-q"]);
    git_ok(&repo, &["config", "user.email", "t@example.com"]);
    git_ok(&repo, &["config", "user.name", "Tester"]);
    std::fs::write(repo.join("base.txt"), "base").expect("write base");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "base"]);
    let base = git_ok(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    let trunk = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"])
        .trim()
        .to_string();

    git_ok(&repo, &["checkout", "-qb", "topic"]);
    std::fs::write(repo.join("topic.txt"), "topic").expect("write topic");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "topic"]);
    git_ok(&repo, &["checkout", "-q", &trunk]);
    std::fs::write(repo.join("trunk.txt"), "trunk").expect("write trunk");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "trunk"]);
    git_ok(
        &repo,
        &[
            "merge",
            "-q",
            "--no-ff",
            "topic",
            "-m",
            "merge topic\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
        ],
    );
    let final_tree = git_ok(&repo, &["rev-parse", "HEAD^{tree}"])
        .trim()
        .to_string();
    let original_parents = git_ok(&repo, &["show", "-s", "--format=%P", "HEAD"]);
    let parent_trees = original_parents
        .split_whitespace()
        .map(|parent| {
            git_ok(&repo, &["rev-parse", &format!("{parent}^{{tree}}")])
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(parent_trees.len(), 2, "fixture must be a two-parent merge");

    let stripped = strip_attribution(
        &repo,
        &Baseline::Commit(base),
        &crate::cancel::Cancel::new(),
        Duration::from_secs(10),
    )
    .await
    .expect("strip merge attribution");

    let rewritten_parents = git_ok(&repo, &["show", "-s", "--format=%P", "HEAD"]);
    let rewritten_parent_trees = rewritten_parents
        .split_whitespace()
        .map(|parent| {
            git_ok(&repo, &["rev-parse", &format!("{parent}^{{tree}}")])
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(stripped, ["merge topic"]);
    assert_eq!(
        git_ok(&repo, &["rev-parse", "HEAD^{tree}"]).trim(),
        final_tree
    );
    assert_eq!(rewritten_parent_trees, parent_trees);
    assert!(!message_of(&repo, "HEAD").unwrap().contains("Claude"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn attribution_strip_refuses_remote_only_lightweight_and_annotated_tags() {
    for (kind, annotated) in [("annotated", true), ("lightweight", false)] {
        let (root, repo, base, credited) = attributed_repo(&format!("remote-tag-{kind}"));
        let remote = root.join("remote.git");
        std::fs::create_dir_all(&remote).expect("create remote");
        git_ok(&remote, &["init", "-q", "--bare"]);
        if kind == "lightweight" {
            let fetch = root.join("fetch-only.git");
            std::fs::create_dir_all(&fetch).expect("create fetch remote");
            git_ok(&fetch, &["init", "-q", "--bare"]);
            git_ok(
                &repo,
                &["remote", "add", "archive", &fetch.to_string_lossy()],
            );
            git_ok(
                &repo,
                &[
                    "remote",
                    "set-url",
                    "--push",
                    "archive",
                    &remote.to_string_lossy(),
                ],
            );
        } else {
            git_ok(
                &repo,
                &["remote", "add", "archive", &remote.to_string_lossy()],
            );
        }
        // The annotated case tags a descendant, so detecting only an exact
        // peeled OID still misses the already-published credited ancestor.
        if annotated {
            std::fs::write(repo.join("later.txt"), "later").expect("write descendant");
            git_ok(&repo, &["add", "-A"]);
            git_ok(&repo, &["commit", "-qm", "later"]);
            git_ok(&repo, &["tag", "-a", "published", "-m", "published"]);
        } else {
            git_ok(&repo, &["tag", "published", &credited]);
        }
        git_ok(
            &repo,
            &[
                "push",
                "-q",
                "archive",
                "refs/tags/published:refs/tags/published",
            ],
        );
        git_ok(&repo, &["tag", "-d", "published"]);
        assert!(
            git(&repo, &["rev-parse", "--verify", "refs/tags/published"]).is_err(),
            "the test accidentally retained its local tag"
        );
        assert!(
            git_ok(&repo, &["branch", "-r", "--contains", &credited])
                .trim()
                .is_empty(),
            "the test accidentally published a branch"
        );

        let expected_head = git_ok(&repo, &["rev-parse", "HEAD"]).trim().to_string();
        let result = strip_attribution(
            &repo,
            &Baseline::Commit(base),
            &crate::cancel::Cancel::new(),
            Duration::from_secs(10),
        )
        .await;
        let actual_head = git_ok(&repo, &["rev-parse", "HEAD"]).trim().to_string();
        assert!(
            result.is_err() && actual_head == expected_head,
            "a remote-only {kind} tag did not protect published history: result={result:?}, \
             HEAD moved from {expected_head} to {actual_head}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[tokio::test]
async fn a_private_local_tag_also_blocks_the_rewrite() {
    let (root, repo, base, credited) = attributed_repo("local-tag");
    git_ok(&repo, &["tag", "private", &credited]);
    let head = git_ok(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    let result = strip_attribution(
        &repo,
        &Baseline::Commit(base),
        &crate::cancel::Cancel::new(),
        Duration::from_secs(10),
    )
    .await;
    assert!(result.is_err(), "a tagged commit was rewritten: {result:?}");
    assert_eq!(git_ok(&repo, &["rev-parse", "HEAD"]).trim(), head);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn a_clean_history_does_not_require_remote_tag_access() {
    let root = std::env::temp_dir().join(format!("bugsleuth-clean-attr-{}", std::process::id()));
    let repo = root.join("repo");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&repo).expect("create repository");
    git_ok(&repo, &["init", "-q"]);
    git_ok(&repo, &["config", "user.email", "t@example.com"]);
    git_ok(&repo, &["config", "user.name", "Tester"]);
    std::fs::write(repo.join("a.txt"), "base").expect("write base");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "base"]);
    let base = git_ok(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    std::fs::write(repo.join("a.txt"), "ordinary fix").expect("write fix");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-qm", "ordinary fix"]);
    git_ok(&repo, &["remote", "add", "offline", "missing-remote"]);

    let result = strip_attribution(
        &repo,
        &Baseline::Commit(base),
        &crate::cancel::Cancel::new(),
        Duration::from_millis(100),
    )
    .await;
    assert_eq!(
        result.expect("a clean history needs no remote query"),
        Vec::<String>::new()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn concurrent_branch_move_is_not_clobbered_by_attribution_strip() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-strip-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test repository");
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run git");
        assert!(output.status.success(), "git {args:?}: {output:?}");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "t@example.com"]);
    run(&["config", "user.name", "Tester"]);
    std::fs::write(dir.join("a.txt"), "base").expect("write base");
    run(&["add", "-A"]);
    run(&["commit", "-qm", "base"]);
    let base = git(&dir, &["rev-parse", "HEAD"])
        .expect("read base")
        .trim()
        .to_string();

    std::fs::write(dir.join("a.txt"), "fix").expect("write fix");
    run(&["add", "-A"]);
    run(&[
        "commit",
        "-qm",
        "fix\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
    ]);
    let expected_head = git(&dir, &["rev-parse", "HEAD"])
        .expect("read expected HEAD")
        .trim()
        .to_string();

    std::fs::write(dir.join("concurrent.txt"), "keep me").expect("write concurrent change");
    run(&["add", "-A"]);
    run(&["commit", "-qm", "concurrent commit"]);
    let concurrent_head = git(&dir, &["rev-parse", "HEAD"])
        .expect("read concurrent HEAD")
        .trim()
        .to_string();
    let branch = git(&dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .expect("read branch")
        .trim()
        .to_string();

    assert!(
        strip_attribution_at(
            &dir,
            &Baseline::Commit(base),
            &branch,
            &expected_head,
            &crate::cancel::Cancel::new(),
            Duration::from_secs(10),
        )
        .await
        .is_err(),
        "moving the branch after HEAD was frozen must abort the rewrite"
    );
    assert_eq!(
        git(&dir, &["rev-parse", "HEAD"])
            .expect("read final HEAD")
            .trim(),
        concurrent_head,
        "the concurrent commit must remain the branch tip"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn attribution_read_errors_fail_closed() {
    let dir =
        std::env::temp_dir().join(format!("bugsleuth-attr-failclosed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    };
    if !run(&["init", "-q"]) {
        return;
    }
    let _ = run(&["config", "user.email", "t@example.com"]);
    let _ = run(&["config", "user.name", "Tester"]);
    let _ = std::fs::write(dir.join("a.txt"), "one");
    let _ = run(&["add", "-A"]);
    if !run(&["commit", "-qm", "base"]) {
        return;
    }

    let missing = "0".repeat(40);
    assert!(message_of(&dir, &missing).is_err());
    assert!(
        refuse_if_published(
            &dir,
            std::slice::from_ref(&missing),
            &crate::cancel::Cancel::new(),
            Duration::from_secs(10),
        )
        .await
        .is_err(),
        "a failed publication query must error, not read as unpublished"
    );
    assert!(attributed_since(&dir, &Baseline::Commit(missing.clone())).is_err());

    let head = git(&dir, &["rev-parse", "HEAD"])
        .expect("head")
        .trim()
        .to_string();
    assert!(
        strip_attribution(
            &dir,
            &Baseline::Commit(missing),
            &crate::cancel::Cancel::new(),
            Duration::from_secs(10),
        )
        .await
        .is_err()
    );
    assert_eq!(
        git(&dir, &["rev-parse", "HEAD"]).expect("head").trim(),
        head,
        "HEAD must be untouched when history could not be inspected"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
