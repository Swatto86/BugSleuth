//! Everything BugSleuth does, minus the way you ask for it.
//!
//! This is the crate that composes the others: it turns a configuration into
//! sweeps, runs them, verifies what comes back, merges it and optionally proves
//! it. `provider`, `verify` and `judge` know nothing of each other; this is
//! where they meet.
//!
//! It exists as a library rather than living inside the command-line binary so
//! that a second front end — the desktop app — runs exactly the same code rather
//! than a parallel implementation of it. A Tauri command should be a
//! deserialize, a call in here, and a serialize.

pub mod brief;
pub mod merge;
pub mod orchestrate;
pub mod plan;
pub mod prove;
pub mod report;
pub mod sweep;
