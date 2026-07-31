//! CLI subprocess adapters.
//!
//! Providers here are *not* HTTP clients. BugSleuth drives the coding CLIs the
//! user is already signed into, so a sweep costs subscription quota rather than
//! metered API credits, and no API key has to exist for the tool to work.
//!
//! Every adapter also accepts an explicit API key. That is deliberate and
//! first-class rather than a fallback: subscription-vs-metered policy for
//! non-interactive CLI use has changed three times in four months, so the day it
//! changes again this needs to be a config edit, not a rewrite.
//!
//! This crate knows nothing about scheduling, judging or storage. It turns a
//! brief into raw, still-untrusted findings.

pub mod claude;
pub mod codex;
mod error;
mod find;
mod json;
pub mod process;

pub use claude::{ClaudeSweep, SweepResult, Usage};
pub use error::ProviderError;
pub use process::{CliOutput, Invocation, ProcessError};
