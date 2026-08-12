//! Keeping the repository under review out of its own review.
//!
//! Every one of these CLIs reads instruction files from the directory it is
//! started in — `AGENTS.md`, `CONTEXT.md`, rules directories — and treats them
//! as its own standing orders. When the directory is a repository you are
//! *reviewing*, that is the repository writing the reviewer's brief.
//!
//! Two vendors have a flag for it. Claude has `--safe-mode` and Codex has
//! `--ignore-rules`, and both are already set. **Kilo has neither**, and
//! `--pure` only skips external plugins. So for Kilo the isolation has to be
//! done to the tree instead — which is possible precisely because Kilo already
//! gets a throwaway worktree rather than the real checkout.
//!
//! This started as a bug that looked like nothing of the sort. Every Kilo sweep
//! of a real repository failed with exit code 1, and the reason turned out to be
//! a 165 KB `CONTEXT.md` in that repository: Kilo loaded it, compacted three
//! times, ran out of context and gave up before reading a line of code. The same
//! command in the same worktree with that one file removed succeeded.
//!
//! The size was the symptom. The problem is that a reviewed repository could
//! shape its own review at all — a file saying "do not report authentication
//! issues" would have been obeyed just as faithfully.

use std::path::Path;

use bugsleuth_verify::Worktree;

/// Files a CLI would read as instructions rather than as code.
///
/// Matched by exact name, case-insensitively, anywhere in the tree — nested
/// `AGENTS.md` is a real convention, not just a root-level one.
const INSTRUCTION_FILES: &[&str] = &[
    "agents.md",
    "agent.md",
    "context.md",
    "claude.md",
    "kilo.md",
    "kimi.md",
    "cursor.md",
    "kilo.json",
    "kilo.jsonc",
    "opencode.json",
    "opencode.jsonc",
    "gemini.md",
    "copilot-instructions.md",
    ".cursorrules",
    ".windsurfrules",
];

/// Directories whose whole contents are instructions.
///
/// `.kilo` is not merely instructions and is the reason this list is
/// load-bearing rather than tidy: a `.kilo/agent/ask.md` in the reviewed tree
/// *redefines the agent BugSleuth sweeps with*, and a repository supplying one
/// with `bash: allow` gets bash back. Measured, not assumed — the same
/// `echo pwned` that the global config denies ran to completion once the
/// worktree contained that file. This is the reviewed repository granting
/// itself permissions its reviewer had refused.
const INSTRUCTION_DIRS: &[&str] = &[
    ".kilo",
    ".kilocode",
    // Kimi discovers agent profiles and skills from project directories, which
    // is the same hole `.kilo` is here for: a profile in the reviewed tree
    // would redefine the agent its own review runs as.
    ".kimi",
    ".kimi-code",
    ".agents",
    ".claude",
    ".cursor",
    ".windsurf",
    ".clinerules",
];

/// Directories never worth walking into, for speed and safety.
const SKIP: &[&str] = &[".git", "target", "node_modules", "dist", "build", "vendor"];

/// Remove instruction files from a throwaway worktree.
///
/// **Only ever call this on a worktree the caller owns and will delete.** It
/// deletes files. Passing a real checkout would edit someone's repository.
///
/// Returns the repo-relative paths removed, so a caller can say what it did.
pub(super) fn strip_agent_instructions(worktree: &Path) -> std::io::Result<Vec<String>> {
    let mut removed = Vec::new();
    strip_in(worktree, worktree, &mut removed)?;
    removed.sort();
    Ok(removed)
}

fn strip_in(dir: &Path, root: &Path, removed: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let metadata = std::fs::symlink_metadata(&path)?;
        let is_link = metadata.file_type().is_symlink() || is_reparse_point(&metadata);

        if INSTRUCTION_DIRS.contains(&name.as_str()) {
            if is_link {
                // Remove the link only — never the target.
                if std::fs::remove_dir(&path).is_err() {
                    std::fs::remove_file(&path)?;
                }
            } else if metadata.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
            removed.push(relative(&path, root));
            continue;
        }

        if is_link {
            continue;
        }

        if metadata.is_dir() {
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            strip_in(&path, root, removed)?;
        } else if INSTRUCTION_FILES.contains(&name.as_str()) {
            std::fs::remove_file(&path)?;
            removed.push(relative(&path, root));
        }
    }
    Ok(())
}

/// Whether a directory entry is a Windows reparse point (junction, mount
/// point, or symlink). `is_symlink()` alone does not catch every junction on
/// every Rust/Windows pairing. Always false off Windows.
#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The checkout an isolated vendor may review, or the reason it may not.
///
/// `None` for a vendor that reads the working tree directly. For one that must
/// be isolated, a throwaway worktree that has been shown to be a *complete*
/// checkout and had the reviewed repository's standing orders taken out of it.
///
/// Both refusals here are gaps the reader would otherwise never hear about: a
/// worktree that could not be created, and one whose submodules `git worktree
/// add` checked out as empty directories. The second returned an ordinary swept
/// result that had reviewed less code than every other lane in the run.
pub(super) fn checkout_for(
    vendor: super::Vendor,
    repo: &std::path::Path,
) -> Result<Option<Worktree>, String> {
    if !vendor.needs_isolation() {
        return Ok(None);
    }
    let worktree = Worktree::create(repo, "HEAD", &format!("sweep-{}", vendor.label())).map_err(
        |error| {
            format!(
                "{} cannot be run read-only, so its sweep needs a throwaway git worktree,                  which could not be created: {error}",
                vendor.label()
            )
        },
    )?;
    // `git worktree add` checks out gitlinks but does not initialize their
    // contents, so an initialized submodule's source is simply absent here while
    // Claude and Codex read it from the main checkout. Recursive initialization
    // would fetch over the network on the user's behalf, which is a separate
    // decision to take deliberately; until then the lane is not run.
    match worktree.has_gitlinks() {
        Ok(true) => {
            return Err(format!(
                "{}'s isolated worktree does not initialize submodules; this lane was not run                  rather than review a partial tree",
                vendor.label()
            ));
        }
        Err(error) => {
            return Err(format!(
                "{}'s isolated worktree could not be checked for submodules, so it might be a                  partial tree: {error}",
                vendor.label()
            ));
        }
        Ok(false) => {}
    }
    strip_agent_instructions(worktree.path()).map_err(|error| {
        format!(
            "{}'s throwaway worktree could not be isolated from project instructions: {error}",
            vendor.label()
        )
    })?;
    Ok(Some(worktree))
}

#[cfg(test)]
#[path = "isolate_strip_tests.rs"]
mod tests;
