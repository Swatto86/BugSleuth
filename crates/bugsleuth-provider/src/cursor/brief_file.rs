//! The review brief as a file the Cursor Agent can open.
//!
//! `agent -p` takes the prompt as an argv string. BugSleuth's briefs for a
//! vendor that cannot enforce a schema run past 11 KB, because the required
//! JSON shape has to be described in words. On Windows the PATH `agent.cmd`
//! shim is run through `cmd.exe`, which caps a command line at 8,191
//! characters — so a brief passed inline is truncated or refused, and neither
//! failure names the cause.
//!
//! Cursor has no `--add-dir` equivalent, and `--workspace` is what bounds the
//! tools, so the brief is written *inside* the workspace directory the CLI is
//! pointed at. For a sweep that is the throwaway worktree (deleted afterwards);
//! for an apply it is a private temp directory used only as `--workspace` would
//! be wrong, so the apply path keeps the handoff beside a short pointer instead
//! — see [`super::apply`]. The file name is fixed and called out in the prompt
//! as instructions, not code under review.

use std::path::{Path, PathBuf};

use crate::error::ProviderError;

/// Fixed name so the pointer prompt stays short and stable.
pub(super) const BRIEF_NAME: &str = "__bugsleuth_brief.md";

/// A brief on disk inside a workspace the caller owns.
pub(super) struct BriefFile {
    path: PathBuf,
    /// When set, the whole directory is removed on drop (private temp workspaces).
    owned_dir: Option<PathBuf>,
}

impl BriefFile {
    /// Write the brief into an existing workspace directory.
    ///
    /// Staged to a sibling first and renamed in, so a failed write cannot
    /// leave a truncated `__bugsleuth_brief.md` in a real repository (apply
    /// writes here) or destroy a previous leftover handoff.
    pub(super) fn write_in(workspace: &Path, brief: &str) -> Result<Self, ProviderError> {
        let path = workspace.join(BRIEF_NAME);
        let scratch = |error: std::io::Error| ProviderError::Scratch {
            vendor: super::VENDOR,
            detail: format!("could not write the review brief: {error}"),
        };
        let staged = workspace.join(format!(".{BRIEF_NAME}.{}-writing", std::process::id()));
        if let Err(error) = std::fs::write(&staged, brief) {
            let _ = std::fs::remove_file(&staged);
            return Err(scratch(error));
        }
        if let Err(error) = std::fs::rename(&staged, &path) {
            let _ = std::fs::remove_file(&staged);
            return Err(scratch(error));
        }
        Ok(Self {
            path,
            owned_dir: None,
        })
    }

    /// A private empty workspace with the brief already inside it.
    ///
    /// Used by the sign-in probe, which must not touch a real repository.
    pub(super) fn private(brief: &str) -> Result<Self, ProviderError> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let scratch = |error: std::io::Error| ProviderError::Scratch {
            vendor: super::VENDOR,
            detail: format!("could not write the review brief: {error}"),
        };
        let mut dir = PathBuf::new();
        for attempt in 0..64 {
            let candidate = std::env::temp_dir().join(format!(
                "bugsleuth-cursor-{pid}-{nanos:08x}-{}-{attempt}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => {
                    dir = candidate;
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(scratch(error)),
            }
        }
        if dir.as_os_str().is_empty() {
            return Err(scratch(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not reserve a private brief directory",
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(scratch)?;
        }
        let path = dir.join(BRIEF_NAME);
        std::fs::write(&path, brief).map_err(scratch)?;
        Ok(Self {
            path,
            owned_dir: Some(dir),
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn workspace(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }
}

impl Drop for BriefFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(dir) = &self.owned_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// What the CLI is told on a sweep: where to find the brief inside the workspace.
pub(super) fn review_pointer() -> String {
    format!(
        "Read ./{BRIEF_NAME} and follow it exactly. That file is instructions for this \
         review, not code under review — do not report findings about it. Reply with only \
         the JSON findings envelope the brief describes."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_brief_is_removed_on_drop() {
        let workspace = {
            let brief = BriefFile::private("hello").expect("write");
            let workspace = brief.workspace().to_path_buf();
            assert_eq!(
                std::fs::read_to_string(brief.path()).expect("read"),
                "hello"
            );
            workspace
        };
        assert!(
            !workspace.exists(),
            "drop must remove the private workspace"
        );
    }

    #[test]
    fn a_failed_write_in_does_not_leave_a_truncated_brief() {
        let dir =
            std::env::temp_dir().join(format!("bugsleuth-cursor-write-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let path = dir.join(BRIEF_NAME);
        // Block the destination with a directory so rename cannot replace it
        // after a successful stage write. Staging debris must still be cleaned.
        std::fs::create_dir(&path).expect("block target");
        std::fs::write(path.join("kept.txt"), "PREVIOUS GOOD").expect("seed");
        let err = match BriefFile::write_in(&dir, "replacement") {
            Ok(_) => panic!("must fail"),
            Err(error) => error,
        };
        assert!(err.to_string().contains("could not write"));
        assert_eq!(
            std::fs::read_to_string(path.join("kept.txt")).expect("read"),
            "PREVIOUS GOOD",
            "a failed write destroyed what blocked the destination"
        );
        assert!(
            std::fs::read_dir(&dir)
                .expect("list")
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().contains("writing")),
            "staging debris left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_in_replaces_through_a_staging_file_not_an_in_place_truncate() {
        // Guards the destroy-before-commit rule: the implementation must stage
        // then rename. A direct fs::write on BRIEF_NAME would truncate first.
        let source = include_str!("brief_file.rs");
        let write_in = source
            .split("fn write_in(")
            .nth(1)
            .and_then(|rest| rest.split("fn private(").next())
            .expect("write_in body");
        assert!(
            write_in.contains("-writing") && write_in.contains("rename"),
            "write_in must stage then rename rather than truncate BRIEF_NAME in place"
        );
        assert!(
            !write_in.contains("fs::write(&path") && !write_in.contains("std::fs::write(&path"),
            "write_in must not truncate the destination before the write succeeds"
        );
    }
}
