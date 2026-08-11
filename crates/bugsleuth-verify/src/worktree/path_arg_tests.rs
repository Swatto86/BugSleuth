//! Git path-boundary behaviour for worktree creation.

use super::*;
use std::path::Path;

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
        raw.starts_with(paths::VERBATIM),
        "precondition: canonicalize should produce the prefix, got {raw}"
    );

    let arg = git_arg(&canonical);
    assert!(!arg.starts_with(paths::VERBATIM), "prefix survived: {arg}");
    assert_eq!(arg, raw.trim_start_matches(paths::VERBATIM));
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
