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
mod testrun;
mod worktree;

pub use anchor::{Rejection, verify_anchor};
pub use console::hide as hide_console_window;
pub use testrun::{Outcome, TestError, TestRun, counts, run as run_tests};
pub use worktree::{Worktree, WorktreeError};
