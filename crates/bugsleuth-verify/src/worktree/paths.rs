//! Converting paths, and reading git's path output, at the git boundary.
//!
//! Split from `worktree.rs` at the hard line cap. All of it is about the gap
//! between what Rust's filesystem calls want and what git accepts or emits:
//! `\\?\` extended-length paths git rejects, `\\?\UNC\` network paths that must
//! not be truncated, and git's NUL-delimited status output.

use std::path::{Path, PathBuf};

/// Windows' verbatim path prefix, which lifts the 260-character limit for Rust's
/// filesystem calls and which git refuses. Named once so the code that adds it,
/// the code that strips it, and the tests agree on the same string — it is easy
/// to mistype by a backslash, and a wrong literal would silently match nothing.
pub(super) const VERBATIM: &str = r"\\?\";
pub(super) const VERBATIM_UNC: &str = r"\\?\UNC\";

/// A path as an argument for `git`, which cannot take the extended-length form.
///
/// The inverse of `long_path`, and needed for the opposite reason. Rust's own
/// filesystem calls want `\\?\` so they are not bound by `MAX_PATH`; git rejects
/// it outright. Every path here comes from `canonicalize`, which on Windows
/// always produces that prefix, so without this every `git worktree add` fails
/// with `could not create leading directories … Invalid argument` — the path is
/// reported back verbatim as `//?/C:/…`, which reads like a malformed URL rather
/// than the deliberate prefix it is.
pub(super) fn git_arg(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    if !cfg!(windows) {
        return text;
    }
    strip_verbatim(&text)
}

/// Strip Windows' verbatim prefix from a path string, preserving a UNC network
/// path.
///
/// The single definition of the rule. [`git_arg`] here and the path validators
/// in the CLI and the desktop app all go through it, because a rule this easy to
/// mistype — one backslash out and the literal silently matches nothing — must
/// not be written three times to drift three ways.
fn strip_verbatim(text: &str) -> String {
    // `\\?\UNC\server\share` is a real network path: dropping only the verbatim
    // prefix would leave `UNC\server\share`, which resolves nowhere.
    if let Some(rest) = text.strip_prefix(VERBATIM_UNC) {
        return format!(r"\\{rest}");
    }
    text.strip_prefix(VERBATIM).unwrap_or(text).to_string()
}

/// A canonicalized path in the form `git` accepts, as a `PathBuf`.
///
/// `canonicalize` returns Windows' extended-length `\\?\` form, which git
/// rejects, and a `\\?\UNC\` network path must become `\\server\share` rather
/// than the relative `UNC\server\share` that dropping only `\\?\` would leave.
/// Shared by the CLI and desktop path validators so all three agree on it.
#[must_use]
pub fn git_path(path: &Path) -> PathBuf {
    PathBuf::from(strip_verbatim(&path.to_string_lossy()))
}

/// The paths in `git status --porcelain -z` output.
///
/// NUL-delimited machine format — no C-quoting of unusual names, no backslash
/// escaping, and no ` -> ` rename arrow to be mistaken for part of a filename. A
/// rename or copy is two records, the new path then the old; the new one is what
/// exists, so the old record is consumed and dropped. The same decoding as
/// `dirty_files` in crates/bugsleuth-engine/src/apply/observed.rs, kept in step
/// by hand because the two crates do not share this parser.
pub(super) fn porcelain_paths(porcelain: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut records = porcelain.split('\0');
    while let Some(record) = records.next() {
        let Some(path) = record.get(3..) else {
            continue;
        };
        let status = &record[..2];
        if status.contains('R') || status.contains('C') {
            records.next();
        }
        if !path.is_empty() {
            out.push(path.to_string());
        }
    }
    out
}
