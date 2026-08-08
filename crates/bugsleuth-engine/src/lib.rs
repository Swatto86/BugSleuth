//! Everything BugSleuth does, minus the way you ask for it.
//!
//! This is the crate that composes the others: it turns a configuration into
//! sweeps, runs them, verifies what comes back, and merges it. `provider`,
//! `verify` and `judge` know nothing of each other; this is where they meet.
//!
//! It exists as a library rather than living inside the command-line binary so
//! that a second front end — the desktop app — runs exactly the same code rather
//! than a parallel implementation of it. A Tauri command should be a
//! deserialize, a call in here, and a serialize.

pub mod apply;
pub mod atomic;
pub mod brief;
pub mod bulk;
pub mod cancel;
mod caveats;
pub mod handoff;
pub mod merge;
/// What each vendor can be asked to run, and how hard.
///
/// Re-exported so the desktop shell keeps its single dependency on this crate:
/// the front ends talk to the engine, and the engine talks to the providers.
pub use bugsleuth_provider::models;

/// Re-exported for the same reason as `models`: the desktop shell reads the
/// judge's output types through the engine rather than depending on the judge
/// crate itself.
pub use bugsleuth_judge::{Cluster, Ranked};

/// Re-exported for the same reason: the desktop shell canonicalizes a repository
/// path through the engine's one dependency rather than taking on the verify
/// crate directly, and the CLI shares the same single definition of the rule.
pub use bugsleuth_verify::git_path;

pub mod orchestrate;
pub mod plan;
pub mod report;
pub mod sweep;
pub mod triage;
