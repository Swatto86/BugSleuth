//! The case these exist for is the one that already happened: a repository
//! whose documentation outweighs its source, and a sweep that died on context
//! with a message naming the model rather than the file responsible.

use super::*;

/// Build a repository on disk. Named per test so two cannot collide, and
/// cleared first so a previous run's files cannot be counted by this one.
fn repo(name: &str, files: &[(&str, usize)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bugsleuth-bulk-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    for (path, size) in files {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&path, "x".repeat(*size)).expect("write");
    }
    dir
}

#[cfg(unix)]
fn symlink_file(original: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn symlink_file(original: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

#[test]
fn a_documentation_file_larger_than_the_code_is_named() {
    // OnTop, in miniature: a teaching book beside a small crate.
    let dir = repo(
        "book",
        &[
            ("src/lib.rs", 4_000),
            ("src/main.rs", 2_000),
            ("BOOK.md", 108 * 1024),
            ("README.md", 900),
        ],
    );

    let bulk = measure(&dir);

    // Guard the guard: a scan that stopped matching would report zero code and
    // satisfy every assertion below about `other` exceeding it.
    assert_eq!(bulk.code, 6_000, "the scan did not read the source files");
    assert!(bulk.worth_saying());

    let warning = caution(&bulk).expect("a 108 KB document should be worth saying");
    assert!(
        warning.contains("BOOK.md"),
        "the warning must name the file, not describe the problem in the abstract: {warning}"
    );
    assert!(
        !warning.contains("README.md"),
        "a 900 byte readme is not the cause of anything: {warning}"
    );
}

#[test]
fn an_ordinary_repository_says_nothing() {
    // The check has to be able to stay quiet, or it is noise that gets ignored
    // exactly when it finally matters.
    let dir = repo(
        "ordinary",
        &[
            ("src/lib.rs", 40_000),
            ("src/app.ts", 20_000),
            ("README.md", 3_000),
        ],
    );

    let bulk = measure(&dir);

    assert_eq!(bulk.code, 60_000);
    assert!(!bulk.worth_saying(), "nothing here is unusual");
    assert_eq!(caution(&bulk), None);
}

#[test]
fn bulk_with_no_single_offender_still_reports() {
    // No file is individually large, but the reviewer is still reading more
    // prose than code. Different shape, same consequence.
    let dir = repo(
        "spread",
        &[
            ("src/lib.rs", 5_000),
            ("a.md", 20_000),
            ("b.md", 20_000),
            ("c.md", 20_000),
        ],
    );

    let bulk = measure(&dir);

    assert!(
        bulk.heavy.is_empty(),
        "no single file is over the threshold"
    );
    assert!(bulk.other > bulk.code);
    assert!(caution(&bulk).is_some(), "the total is the finding here");
}

#[test]
fn build_output_is_not_counted() {
    // `target` can be gigabytes and is not what anyone is reviewing. Counting
    // it would make the warning fire on every Rust repository, which is the
    // same as it never firing.
    let dir = repo(
        "skip",
        &[
            ("src/lib.rs", 10_000),
            ("target/debug/huge.bin", 500 * 1024),
            ("node_modules/pkg/index.js", 400 * 1024),
        ],
    );

    let bulk = measure(&dir);

    assert_eq!(bulk.code, 10_000, "node_modules was counted as source");
    assert_eq!(bulk.other, 0, "target was counted");
    assert!(!bulk.worth_saying());
}

#[test]
fn an_empty_repository_does_not_trigger_on_the_ratio() {
    // `other > code` is trivially true when there is no code at all, and a
    // warning about a repository with nothing in it helps nobody.
    let dir = repo("empty", &[("notes.txt", 100)]);
    let bulk = measure(&dir);
    assert_eq!(bulk.code, 0);
    assert!(!bulk.worth_saying());
}

#[test]
fn a_lockfile_is_not_named() {
    // Found by running the check on a real repository rather than by writing a
    // test: it named Cargo.lock at 106 KB beside the document that had actually
    // broken a sweep. A lockfile is generated, nobody reviews it, and there is
    // nothing useful to do about it - so naming it only teaches the reader to
    // skim the warning.
    let dir = repo(
        "lockfile",
        &[
            ("src/lib.rs", 10_000),
            ("Cargo.lock", 106 * 1024),
            ("BOOK.md", 108 * 1024),
        ],
    );

    let bulk = measure(&dir);
    let warning = caution(&bulk).expect("the book is still worth saying");

    assert!(
        warning.contains("BOOK.md"),
        "the real cause must still be named: {warning}"
    );
    assert!(
        !warning.contains("Cargo.lock"),
        "a generated lockfile is noise in this warning: {warning}"
    );
    assert_eq!(
        bulk.other,
        108 * 1024,
        "the lockfile was counted in the total"
    );
}

#[test]
fn a_symlink_to_a_file_outside_the_repository_is_not_counted() {
    // A reviewed repository can contain a symlink that points outside the tree.
    // `walk` must not follow it or its target's size would inflate the report.
    let dir = repo("symlink", &[("src/lib.rs", 10_000)]);
    let outside = dir.with_extension("outside-file");
    std::fs::write(&outside, "x".repeat(100_000)).expect("write outside target");

    let link = dir.join("link.md");
    if let Err(e) = symlink_file(&outside, &link) {
        eprintln!("skipping symlink test: {e}");
        return;
    }

    let bulk = measure(&dir);

    assert_eq!(
        bulk.other, 0,
        "the symlinked file outside the repo must not be counted: other={}",
        bulk.other
    );
    assert!(
        !link.exists() || !bulk.heavy.iter().any(|(p, _)| p == "link.md"),
        "the symlinked path must not be named in the warning"
    );
}
