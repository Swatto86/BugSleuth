//! Turning a model's claim into evidence.
//!
//! BugSleuth exists for someone who cannot check a finding by reading the code.
//! To them a confident hallucination and a real defect are indistinguishable, so
//! every mechanical filter that can separate the two is worth having.
//!
//! This crate owns the cheapest of those filters: a finding must quote code that
//! actually exists in the file it names. It rejects invented file paths,
//! invented code, and code quoted from a different repository — the failure
//! modes that produce the most convincing false positives.

mod anchor;
mod console;
mod worktree;

pub use anchor::{Rejection, checked_repo_file, verify_anchor};
pub use console::hide as hide_console_window;
pub use worktree::{
    Worktree, WorktreeError, git_path, validate_repository_identity, worktree_roots,
};
