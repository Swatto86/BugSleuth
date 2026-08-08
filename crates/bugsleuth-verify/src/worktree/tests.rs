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

        // Imitate what a build leaves behind inside an isolated-sweep worktree:
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

/// A directory link inside a repository, or `None` when the OS refuses.
///
/// Mirrors `link_dir` in `anchor/tests.rs`: a Windows junction, which needs no
/// elevation, or a Unix symlink.
#[cfg(windows)]
fn link_dir(target: &Path, at: &Path) -> Option<()> {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(at)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    status.success().then_some(())
}
#[cfg(unix)]
fn link_dir(target: &Path, at: &Path) -> Option<()> {
    std::os::unix::fs::symlink(target, at).ok()
}

/// A committed `.bugsleuth-worktrees` link must not let orphan cleanup reach
/// outside the repository and recursively delete an attacker-chosen directory.
#[test]
fn worktree_creation_refuses_a_linked_container_without_deleting_its_target() {
    let Some(repo) = temp_repo("linked-container") else {
        // No usable git here; the pure-logic tests still cover the rest.
        return;
    };

    // A directory outside the repository, holding something worth losing.
    let target = std::env::temp_dir()
        .join("bugsleuth-worktree-link-target")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(long_path(&target));
    let victim = target.join("victim");
    if std::fs::create_dir_all(&victim).is_err() {
        let _ = std::fs::remove_dir_all(long_path(&repo));
        return;
    }
    let keep = victim.join("keep.txt");
    let _ = std::fs::write(&keep, "do not delete\n");

    // Point the worktree container at that external directory. Git never lists
    // its children as worktrees, so before the fix each is treated as wreckage
    // and recursively removed.
    let container = repo.join(".bugsleuth-worktrees");
    let Some(()) = link_dir(&target, &container) else {
        eprintln!("skipped: this OS would not create a directory link");
        let _ = std::fs::remove_dir_all(long_path(&repo));
        let _ = std::fs::remove_dir_all(long_path(&target));
        return;
    };

    let result = Worktree::create(&repo, "HEAD", "security");
    assert!(
        matches!(result, Err(WorktreeError::UnsafeWorktreeRoot { .. })),
        "a linked worktree container was accepted instead of refused: {result:?}"
    );
    assert!(
        keep.exists(),
        "orphan cleanup followed the link and deleted the external target at {}",
        keep.display()
    );

    let _ = std::fs::remove_dir_all(long_path(&repo));
    let _ = std::fs::remove_dir_all(long_path(&target));
}

/// A reviewed repository can commit a directory under the worktree container.
/// Orphan cleanup must not mistake it for wreckage and delete it, or the code
/// being reviewed can delete files from the user's working tree — the one thing
/// the module promises never to do.
#[test]
fn committed_content_under_the_worktree_container_is_never_deleted() {
    let Some(repo) = temp_repo("committed-container") else {
        // No usable git here; the pure-logic tests still cover the rest.
        return;
    };
    let committed = repo.join(".bugsleuth-worktrees").join("not-ours");
    if std::fs::create_dir_all(&committed).is_err()
        || std::fs::write(committed.join("keep.txt"), "do not delete\n").is_err()
    {
        let _ = std::fs::remove_dir_all(long_path(&repo));
        return;
    }
    let _ = git(&repo, &["add", "-A"]);
    let _ = git(&repo, &["commit", "-qm", "ship a container directory"]);

    let _ = Worktree::create(&repo, "HEAD", "security");

    assert!(
        committed.join("keep.txt").exists(),
        "orphan cleanup deleted repository-controlled content at {}",
        committed.display()
    );
    let _ = std::fs::remove_dir_all(long_path(&repo));
}

/// A renamed file must be reported as the name that exists, not as the raw
#[test]
fn git_paths_preserve_unicode_and_arrow() {
    // `-z` output is raw: `café.rs` is not C-quoted to `"caf\303\251.rs"`, a
    // name containing ` -> ` is not truncated as rename syntax, and a literal
    // backslash is not turned into a slash. A rename is the new path then the
    // old, as two records. The arrow case cannot be a real file on Windows
    // (`>` is forbidden), so the parser is exercised directly.
    let porcelain = concat!(
        " M src/café.rs\0",
        "?? a -> b.rs\0",
        "?? weird\\name.rs\0",
        "R  dst.rs\0src.rs\0",
    );
    assert_eq!(
        porcelain_paths(porcelain),
        ["src/café.rs", "a -> b.rs", "weird\\name.rs", "dst.rs"]
    );
    // A clean tree, and a status record with no path, read as empty.
    assert!(porcelain_paths("").is_empty());
    assert!(porcelain_paths("?? \0").is_empty());
}

/// porcelain `old -> new` string, which is not a path and locates nothing.
#[test]
fn changed_files_reports_the_new_name_of_a_rename_not_the_arrow_string() {
    let Some(repo) = temp_repo("rename") else {
        // No usable git here; the pure-logic tests still cover the rest.
        return;
    };
    let Ok(worktree) = Worktree::create(&repo, "HEAD", "correctness") else {
        let _ = std::fs::remove_dir_all(long_path(&repo));
        return;
    };
    // a.txt is committed by temp_repo, so a rename shows in `-z` output as the
    // two records `R  b.txt\0a.txt\0`.
    if git(worktree.path(), &["mv", "a.txt", "b.txt"]).is_err() {
        let _ = std::fs::remove_dir_all(long_path(&repo));
        return;
    }

    let changed = worktree.changed_files().unwrap_or_default();
    assert_eq!(
        changed,
        ["b.txt"],
        "a rename was not decoded to the name that exists",
    );
    assert!(
        !changed.iter().any(|name| name.contains(" -> ")),
        "the raw porcelain arrow string leaked into the changed-file list: {changed:?}",
    );

    drop(worktree);
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

#[test]
fn unc_verbatim_path() {
    // The bug: `canonicalize` renders a network path as `\\?\UNC\server\share`,
    // and dropping only the `\\?\` prefix leaves the relative `UNC\server\share`
    // — git then runs against the current directory, not the network path. The
    // conversion is pure string work, so it is exercised on every platform.
    assert_eq!(
        git_path(Path::new(r"\\?\UNC\server\share\repo")).to_string_lossy(),
        r"\\server\share\repo"
    );
    // An ordinary verbatim drive path loses only the prefix.
    assert_eq!(
        git_path(Path::new(r"\\?\C:\repo")).to_string_lossy(),
        r"C:\repo"
    );
    // A path with neither prefix is passed through untouched.
    assert_eq!(
        git_path(Path::new(r"C:\plain\path")).to_string_lossy(),
        r"C:\plain\path"
    );
}

#[test]
fn a_git_argument_never_carries_the_extended_length_prefix() {
    // `canonicalize` always produces `\?\` on Windows, and git rejects it:
    // `git worktree add` failed with "could not create leading directories …
    // Invalid argument" for every Kilo sweep. Rust's own filesystem calls want
    // the prefix, so it is stripped at the git boundary rather than not added.
    if !cfg!(windows) {
        return;
    }
    // Derived from `canonicalize` rather than hand-written: the prefix is easy
    // to mistype in a literal, and a wrong literal would test a string the real
    // code never sees. This is exactly what `Worktree::create` passes in.
    let canonical = Path::new(".").canonicalize().expect("canonicalize");
    let raw = canonical.to_string_lossy().into_owned();
    assert!(
        raw.starts_with(VERBATIM),
        "precondition: canonicalize should produce the prefix, got {raw}"
    );

    let arg = git_arg(&canonical);
    assert!(!arg.starts_with(VERBATIM), "prefix survived: {arg}");
    assert_eq!(arg, raw.trim_start_matches(VERBATIM));
    // An empty or truncated result would satisfy "no prefix" too.
    assert!(arg.contains(":\\"), "not a usable drive path: {arg}");

    // An ordinary path is passed through untouched.
    assert_eq!(git_arg(Path::new(r"C:\plain\path")), r"C:\plain\path");
}

#[test]
fn a_worktree_of_this_repository_is_actually_created() {
    // The other creation tests bail out with `return` when `Worktree::create`
    // fails, so they passed throughout the bug above — a skip is not a pass.
    // This one runs against the repository the tests live in, which is a real
    // git checkout with a canonicalised, `\?\`-prefixed path, and fails loudly.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    if !repo.join(".git").exists() {
        return;
    }
    let worktree = Worktree::create(repo, "HEAD", "prefix-check")
        .unwrap_or_else(|e| panic!("creating a worktree of the checkout failed: {e}"));
    assert!(worktree.path().exists(), "worktree path does not exist");
}
