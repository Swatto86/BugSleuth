//! Replacing a file without destroying what was there first.
//!
//! `fs::write` truncates before it writes. Every failure after that point — a
//! full disk, a killed process, a permission change between the two calls —
//! leaves an empty or half-written file where a good one had been. For the
//! things this program writes that is not a small loss: a run report, a fix
//! prompt or a settings file, and in the first two cases tens of minutes of
//! paid sweeping that cannot be recovered by rerunning something cheap.
//!
//! **This is one function because it was three.** The destroy-before-commit
//! defect was found by BugSleuth reviewing itself, fixed in the run reports,
//! found again in the fix prompt, fixed again, found again in the settings, and
//! fixed a third time — each fix an independent copy of the same six lines. The
//! command line still had none of them and truncated the user's `--json-out`
//! and `--patch-out` in place. A rule written down three times is a rule with
//! three chances to be missed somewhere else, which is exactly what happened,
//! so a test below asserts nothing writes a durable file any other way.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes staging files written by one process, as the id cannot.
///
/// Two writes in one process can be in flight at once — the orchestrator writes
/// a report per sweep and sweeps run concurrently — so the process id alone
/// would let two of them share a temporary file.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path`, leaving the previous file untouched on failure.
///
/// The content goes to a hidden sibling first and is renamed in. A rename
/// within a directory is atomic on both platforms this ships to, so a reader
/// sees either the whole old file or the whole new one and never a truncated
/// mixture. A failed rename takes the temporary file with it, rather than
/// leaving a `.writing` file beside the real ones for the next reader to
/// puzzle over.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write_with(path, |staged| std::fs::write(staged, contents))
}

/// The body of [`write`] with the staging write injected, so a test can force a
/// write that creates and partially fills the file before it fails.
fn write_with<W>(path: &Path, stage: W) -> io::Result<()>
where
    W: FnOnce(&Path) -> io::Result<()>,
{
    let staged = staged_path(path)?;
    // The staging file is cleaned on *every* exit after it is named, the failed
    // staging write included: `fs::write` may create and partially fill the file
    // before it fails, and returning straight through `?` used to leave that
    // unique `.writing` file behind for good — one more every time the disk was
    // near full. The original error is preserved; a cleanup that also fails is
    // ignored rather than allowed to mask it.
    if let Err(error) = stage(&staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&staged, path) {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    Ok(())
}

/// Whether a name is one of this module's temporaries.
///
/// Anything reading a directory of reports has to skip these, since a sweep
/// running alongside will have one on disk.
pub fn is_staged(name: &str) -> bool {
    name.ends_with(".writing")
}

/// The sibling to stage into: same directory, so the rename is a rename.
///
/// Crossing a filesystem turns `rename` into a copy that can fail halfway,
/// which is the failure this module exists to prevent.
fn staged_path(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} names no file to write", path.display()),
        )
    })?;
    // Unique per writer. The staging name used to be `.{name}.writing` and
    // nothing else, so two processes replacing the same report wrote to one
    // temporary file at the same time: each rename then succeeded, and the file
    // that survived could hold the other writer's bytes. An atomic write that
    // publishes a mixture of two contents is worse than no atomic write, since
    // the whole point is that a reader never sees one.
    //
    // The worktree module learned this an hour earlier and this module was
    // written without it — the same rule, missed in the second place, which is
    // the class this codebase keeps finding in itself.
    let mut staged = std::ffi::OsString::from(".");
    staged.push(name);
    staged.push(format!(
        ".{}-{}.writing",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    Ok(path.with_file_name(staged))
}

#[cfg(test)]
#[path = "atomic/tests.rs"]
mod tests;
