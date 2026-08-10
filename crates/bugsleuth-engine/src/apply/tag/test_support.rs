use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

pub(super) fn git_ok(dir: &Path, args: &[&str]) -> String {
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

pub(super) fn published(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, String) {
    published_on(tag, "origin")
}

pub(super) fn published_on(
    tag: &str,
    remote_name: &str,
) -> (std::path::PathBuf, std::path::PathBuf, String) {
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
        &["remote", "add", remote_name, &remote.to_string_lossy()],
    );
    let branch = git_ok(&repo, &["symbolic-ref", "--short", "HEAD"]);
    git_ok(&repo, &["push", "-q", "-u", remote_name, &branch]);
    (repo, remote, remote_name.to_string())
}

pub(super) fn remote_tags(remote: &Path) -> Vec<String> {
    git_ok(remote, &["tag", "--list"])
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}
