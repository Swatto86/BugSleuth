//! Ownership checks for throwaway worktree names.

use super::*;

fn temp_repo(name: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir()
        .join("bugsleuth-worktree-ownership")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(long_path(&dir));
    std::fs::create_dir_all(&dir).ok()?;
    git(&dir, &["init", "-q"]).ok()?;
    git(&dir, &["config", "user.email", "t@example.invalid"]).ok()?;
    git(&dir, &["config", "user.name", "test"]).ok()?;
    std::fs::write(dir.join("a.txt"), "hello\n").ok()?;
    git(&dir, &["add", "-A"]).ok()?;
    git(&dir, &["commit", "-qm", "base"]).ok()?;
    Some(dir)
}

fn user_commit(repo: &Path) -> String {
    let base = git(repo, &["rev-parse", "HEAD"]).expect("read HEAD");
    let tree = git(repo, &["rev-parse", "HEAD^{tree}"]).expect("read tree");
    git(
        repo,
        &[
            "commit-tree",
            tree.trim(),
            "-p",
            base.trim(),
            "-m",
            "user work",
        ],
    )
    .expect("create user commit")
    .trim()
    .to_string()
}

fn create_branch(repo: &Path, branch: &str, oid: &str) {
    git(repo, &["update-ref", &format!("refs/heads/{branch}"), oid]).expect("create user branch");
}

fn branch_oid(repo: &Path, branch: &str) -> String {
    git(repo, &["rev-parse", &format!("refs/heads/{branch}")])
        .expect("read user branch")
        .trim()
        .to_string()
}

#[test]
fn a_user_branch_with_a_numeric_suffix_is_never_deleted() {
    let Some(repo) = temp_repo("numeric-branch") else {
        return;
    };
    let user_oid = user_commit(&repo);
    let branch = "bugsleuth/release-2026-1";
    create_branch(&repo, branch, &user_oid);

    let worktree =
        Worktree::create(&repo, "HEAD", "trigger-cleanup").expect("create isolated worktree");
    assert_eq!(
        branch_oid(&repo, branch),
        user_oid,
        "a branch name is not proof that BugSleuth owns its commits"
    );

    drop(worktree);
    let _ = std::fs::remove_dir_all(long_path(&repo));
}

#[test]
fn a_preexisting_exact_name_collision_is_not_deleted() {
    let Some(repo) = temp_repo("exact-collision") else {
        return;
    };
    let user_oid = user_commit(&repo);
    let root = repo.join(".bugsleuth-worktrees");
    let first = NEXT.load(Ordering::Relaxed);
    let mut collisions = Vec::new();

    // Other worktree tests may allocate counters in parallel. Cover more than
    // the whole suite can consume so this remains a real collision under the
    // ordinary parallel test runner.
    for counter in first..first + 64 {
        let slug = format!("collision-{}-{counter}", std::process::id());
        let branch = format!("{PREFIX}{slug}");
        let path = root.join(&slug);
        std::fs::create_dir_all(&path).expect("create colliding path");
        std::fs::write(path.join("keep.txt"), "user data\n").expect("write marker");
        create_branch(&repo, &branch, &user_oid);
        collisions.push((branch, path));
    }

    let worktree =
        Worktree::create(&repo, "HEAD", "collision").expect("create worktree beside collisions");
    for (branch, path) in &collisions {
        assert_eq!(branch_oid(&repo, branch), user_oid, "{branch} was replaced");
        assert!(
            path.join("keep.txt").exists(),
            "the colliding path {} was deleted",
            path.display()
        );
    }

    drop(worktree);
    let _ = std::fs::remove_dir_all(long_path(&repo));
}
