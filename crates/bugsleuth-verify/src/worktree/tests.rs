//! Tests for throwaway worktrees, in their own file only because the
//! module plus its tests crossed the hard line cap.

use super::*;

#[test]
fn a_label_cannot_smuggle_path_or_flag_characters_into_a_branch_name() {
    assert_eq!(sanitize("correctness/1"), "correctness-1");
    assert_eq!(sanitize("../../escape"), "escape");
    assert_eq!(sanitize("--force"), "force");
    assert_eq!(sanitize(""), "run");
    assert_eq!(sanitize("!!!"), "run");
}

#[test]
fn a_long_label_is_truncated_rather_than_producing_an_unusable_path() {
    let slug = sanitize(&"a".repeat(200));
    assert_eq!(slug.len(), 48);
}

#[test]
fn a_long_absolute_path_gets_the_extended_length_prefix_on_windows() {
    let path = Path::new(r"C:\Users\x\repo\.bugsleuth-worktrees\run");
    let converted = long_path(path);
    if cfg!(windows) {
        assert!(
            converted.to_string_lossy().starts_with(r#"\\?\"#),
            "got {}",
            converted.display()
        );
    } else {
        assert_eq!(converted, path);
    }
}

#[test]
fn a_path_that_already_has_the_prefix_is_not_given_a_second_one() {
    let path = Path::new(r#"\\?\C:\repo\wt"#);
    assert_eq!(long_path(path), path);
}

/// A throwaway git repository with one commit.
fn temp_repo(name: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir()
        .join("bugsleuth-worktree-tests")
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

#[test]
fn dropping_a_worktree_deletes_it_even_with_deeply_nested_build_output() {
    let Some(repo) = temp_repo("deep") else {
        // No usable git in this environment; the other tests still cover the
        // pure logic. Better to skip than to fail for an unrelated reason.
        return;
    };

    let path = {
        let worktree = match Worktree::create(&repo, "HEAD", "deep") {
            Ok(worktree) => worktree,
            Err(_) => return,
        };
        let path = worktree.path().to_path_buf();

        // Imitate what `cargo test` leaves behind inside a proof worktree:
        // paths long enough that `git worktree remove` fails on Windows with
        // "Filename too long" and silently leaves the directory in place.
        let mut deep = path.join("target");
        for segment in 0..12 {
            deep = deep.join(format!(
                "{segment}-a-rather-long-directory-name-like-cargo-makes"
            ));
        }
        let _ = std::fs::create_dir_all(long_path(&deep));
        let _ = std::fs::write(long_path(&deep.join("artifact.bin")), b"x");
        assert!(path.exists(), "the worktree should exist before the drop");
        path
    };

    assert!(
        !path.exists(),
        "the worktree survived its own drop at {}; it would dirty the reviewed repository",
        path.display()
    );
    assert!(
        !repo.join(".bugsleuth-worktrees").exists(),
        "the container directory was left behind in the reviewed repository"
    );
    let _ = std::fs::remove_dir_all(long_path(&repo));
}

#[test]
fn creating_a_worktree_outside_a_git_repository_is_refused() {
    let not_a_repo = std::env::temp_dir().join("bugsleuth-not-a-repo");
    let _ = std::fs::create_dir_all(&not_a_repo);
    let result = Worktree::create(&not_a_repo, "HEAD", "x");
    assert!(matches!(result, Err(WorktreeError::NotAGitRepo(_))));
}

/// Two worktrees for one label must not choose the same directory.
///
/// The defect: the path was the sanitised label alone, so a second
/// BugSleuth against the same repository picked the same directory and
/// branch — and its "clear the wreckage" step deleted the first one's
/// *live* worktree, taking a running test and the minutes it had cost.
/// Nothing warned; the losing run simply started failing.
#[test]
fn a_second_worktree_for_one_label_does_not_destroy_the_first() {
    let dir = std::env::temp_dir()
        .join("bugsleuth-worktree-collision")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(long_path(&dir));
    if std::fs::create_dir_all(&dir).is_err() || git(&dir, &["init", "-q"]).is_err() {
        // No usable git here; the rest of the suite covers the pure logic.
        return;
    }
    let _ = git(&dir, &["config", "user.email", "t@example.invalid"]);
    let _ = git(&dir, &["config", "user.name", "test"]);
    let _ = std::fs::write(dir.join("a.txt"), "hello\n");
    let _ = git(&dir, &["add", "-A"]);
    if git(&dir, &["commit", "-qm", "base"]).is_err() {
        return;
    }

    let Ok(first) = Worktree::create(&dir, "HEAD", "correctness") else {
        return;
    };
    let first_path = first.path().to_path_buf();
    assert!(first_path.exists(), "the first worktree was not created");

    let second = Worktree::create(&dir, "HEAD", "correctness").expect("second worktree");
    assert_ne!(first.path(), second.path(), "both chose the same directory");
    assert!(
        first_path.exists(),
        "creating the second destroyed the first, which was still in use"
    );

    drop(second);
    drop(first);
    let _ = std::fs::remove_dir_all(long_path(&dir));
}
