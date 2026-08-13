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
//! — see [`super::apply`]. The generated file is called out in the prompt as
//! instructions, not code under review.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::ProviderError;

static NEXT_BRIEF: AtomicU64 = AtomicU64::new(0);

/// A brief on disk inside a workspace the caller owns.
pub(super) struct BriefFile {
    path: PathBuf,
    /// When set, the whole directory is removed on drop (private temp workspaces).
    owned_dir: Option<PathBuf>,
}

impl BriefFile {
    /// Write the brief into an existing workspace directory.
    ///
    /// The name is reserved exclusively, so neither writing nor Drop can touch
    /// a repository file the user already owns.
    pub(super) fn write_in(workspace: &Path, brief: &str) -> Result<Self, ProviderError> {
        let scratch = |error: std::io::Error| ProviderError::Scratch {
            vendor: super::VENDOR,
            detail: format!("could not write the review brief: {error}"),
        };
        for _ in 0..64 {
            let id = NEXT_BRIEF.fetch_add(1, Ordering::Relaxed);
            let path = workspace.join(format!("__bugsleuth_brief-{}-{id}.md", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(brief.as_bytes()) {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Err(scratch(error));
                    }
                    return Ok(Self {
                        path,
                        owned_dir: None,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(scratch(error)),
            }
        }
        Err(scratch(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique Cursor brief",
        )))
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
        let path = dir.join("brief.md");
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

/// What the CLI is told: where to find the brief inside the workspace.
pub(super) fn review_pointer(brief: &Path) -> String {
    format!(
        "@{}",
        brief
            .file_name()
            .unwrap_or(brief.as_os_str())
            .to_string_lossy()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preexisting_repository_file_is_never_replaced_or_deleted() {
        let dir = std::env::temp_dir().join(format!(
            "bugsleuth-cursor-owned-brief-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let original = dir.join("__bugsleuth_brief.md");
        std::fs::write(&original, "user content").expect("seed user file");

        let generated = {
            let brief = BriefFile::write_in(&dir, "generated brief").expect("write brief");
            let generated = brief.path().to_path_buf();
            assert_ne!(generated, original, "the user file was adopted as scratch");
            assert_eq!(
                std::fs::read_to_string(&original).expect("read user file"),
                "user content"
            );
            let pointer = review_pointer(brief.path());
            assert!(
                pointer.contains(
                    generated
                        .file_name()
                        .expect("generated brief has a name")
                        .to_string_lossy()
                        .as_ref()
                ),
                "the pointer does not name the generated brief: {pointer}"
            );
            generated
        };

        assert_eq!(
            std::fs::read_to_string(&original).expect("read user file after drop"),
            "user content"
        );
        assert!(!generated.exists(), "the generated brief survived Drop");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
}
