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

/// Write `contents` to `path`, leaving the previous file untouched on failure.
///
/// The content goes to a hidden sibling first and is renamed in. A rename
/// within a directory is atomic on both platforms this ships to, so a reader
/// sees either the whole old file or the whole new one and never a truncated
/// mixture. A failed rename takes the temporary file with it, rather than
/// leaving a `.writing` file beside the real ones for the next reader to
/// puzzle over.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let staged = staged_path(path)?;
    std::fs::write(&staged, contents)?;
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
    let mut staged = std::ffi::OsString::from(".");
    staged.push(name);
    staged.push(".writing");
    Ok(path.with_file_name(staged))
}

#[cfg(test)]
#[path = "atomic/tests.rs"]
mod tests;
