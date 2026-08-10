//! Handing Kimi a brief it cannot be given as an argument.
//!
//! `kimi --prompt` takes the prompt as an **argv string**; there is no stdin
//! form and no `--prompt-file`. BugSleuth's briefs for a vendor that cannot
//! enforce a schema run 11,024–12,603 characters, because the required JSON
//! shape has to be described in words rather than passed as a schema.
//!
//! That is not something to push through argv. Windows caps a command line at
//! 32,767 characters, and an npm-installed `kimi.cmd` shim is run by Rust
//! through `cmd.exe`, which caps it at 8,191 — so on that install route a 12 KB
//! brief is truncated or the spawn is refused outright, and neither failure
//! says anything about the real cause. The official installer ships a native
//! executable that would survive it today, which is precisely the kind of
//! difference that should not decide whether a review runs.
//!
//! So the brief is written to a file and the prompt points at it. The file goes
//! in a private temp directory rather than in the worktree: the worktree *is*
//! the code under review, and a review brief sitting inside it is one more file
//! the model can find, quote and anchor a finding to.

use std::path::{Path, PathBuf};

use crate::error::ProviderError;

/// A brief on disk, deleted when it goes out of scope.
///
/// Held for the length of the invocation. Dropped on every exit including a
/// panic or an early `?`, so a killed sweep cannot leave briefs accumulating in
/// the temp directory.
pub(super) struct BriefFile {
    dir: PathBuf,
    path: PathBuf,
}

impl BriefFile {
    pub(super) fn write(brief: &str) -> Result<Self, ProviderError> {
        // Unique per process *and* per call: two lanes sweep concurrently, and
        // a shared name would have one overwrite the other's brief mid-run.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bugsleuth-kimi-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let scratch = |error: std::io::Error| ProviderError::Scratch {
            vendor: super::VENDOR,
            detail: format!("could not write the review brief: {error}"),
        };
        std::fs::create_dir_all(&dir).map_err(scratch)?;
        let path = dir.join("brief.md");
        std::fs::write(&path, brief).map_err(scratch)?;
        Ok(Self { dir, path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// The directory to grant the session, so the brief is readable at all.
    ///
    /// Kimi's workspace is its working directory, which is the worktree. The
    /// brief deliberately lives outside that, so it needs `--add-dir`.
    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for BriefFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What Kimi is actually told, which is where to find the rest.
///
/// Deliberately short and free of quotes, backslashes and newlines beyond the
/// path itself: this is the one string that still crosses the command line.
pub(super) fn pointer(brief: &Path) -> String {
    format!(
        "Read the file at {} and carry out the code review it describes, exactly as written. \
         It specifies the JSON object you must reply with. Reply with that JSON and nothing else.",
        brief.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prompt that crosses the command line must stay small.
    ///
    /// `cmd.exe` caps a command line at 8,191 characters and Rust runs the
    /// `.cmd` shim through it, so a brief passed inline would be truncated or
    /// refused. The pointer is the whole point of this module: assert it is
    /// nowhere near the cap even though the brief it replaces is not.
    #[test]
    fn the_prompt_that_reaches_the_command_line_is_tiny() {
        let brief = "x".repeat(12_603);
        let file = BriefFile::write(&brief).expect("write brief");
        let prompt = pointer(file.path());
        assert!(
            prompt.len() < 1_024,
            "the prompt is {} chars; a brief-sized argument does not survive cmd.exe",
            prompt.len()
        );
        // And the brief really is on disk in full, so the pointer points at
        // something — a short prompt naming a missing file would "pass" this.
        assert_eq!(
            std::fs::read_to_string(file.path()).expect("read back"),
            brief
        );
    }

    /// Two concurrent lanes must not share one brief path.
    #[test]
    fn each_brief_gets_its_own_file() {
        let first = BriefFile::write("first").expect("write");
        let second = BriefFile::write("second").expect("write");
        assert_ne!(first.path(), second.path());
        assert_eq!(
            std::fs::read_to_string(first.path()).expect("read"),
            "first",
            "the second brief overwrote the first"
        );
    }

    /// A killed sweep must not leave briefs behind.
    #[test]
    fn the_brief_is_removed_when_the_sweep_ends() {
        let path = {
            let file = BriefFile::write("temporary").expect("write");
            let path = file.path().to_path_buf();
            assert!(path.is_file(), "the brief was never written");
            path
        };
        assert!(!path.exists(), "the brief outlived its sweep at {path:?}");
    }
}
