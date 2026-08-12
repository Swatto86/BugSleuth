//! CLI subprocess adapters.
//!
//! Providers here are *not* HTTP clients. BugSleuth drives the coding CLIs the
//! user is already signed into, so a sweep costs subscription quota rather than
//! metered API credits, and no API key has to exist for the tool to work.
//!
//! The Claude adapter also accepts an explicit Anthropic API key. That is
//! deliberate and first-class rather than a fallback: subscription-vs-metered
//! policy for non-interactive CLI use has changed three times in four months, so
//! the day it changes again this needs to be a config edit, not a rewrite. Codex
//! and Kilo take no key here and use the session configured in their own CLI —
//! saying "every adapter" was how `--use-api-key` came to be accepted for a
//! sweep that could never receive one.
//!
//! This crate knows nothing about scheduling, judging or storage. It turns a
//! brief into raw, still-untrusted findings.

pub mod claude;
pub mod codex;
pub mod cursor;
mod error;
mod find;
mod json;
pub mod kilo;
pub mod kimi;
pub mod models;
pub mod process;
pub mod signin;

pub use claude::{ClaudeSweep, SweepResult, Usage};
pub use error::ProviderError;
pub use process::{CliOutput, Invocation, ProcessError};
