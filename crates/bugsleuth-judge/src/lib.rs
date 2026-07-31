//! Merging findings from several models into one ranked list.
//!
//! Two models sweeping the same lane will report the same defect in different
//! words, and will also report genuinely different defects on adjacent lines.
//! Telling those two situations apart is the whole job.
//!
//! This crate deliberately does **not** ask a model to do it. Clustering by
//! anchor and wording is deterministic, free, instant, and testable against real
//! data — and a model-based adjudicator that cannot beat it is not worth its
//! quota. If one is added later it should be measured against this baseline, not
//! assumed to be better.
//!
//! It also knows nothing about how findings were produced. `provider` does not
//! exist as far as this crate is concerned.

mod cluster;
mod rank;
mod similarity;

pub use cluster::{Cluster, cluster};
pub use rank::{Ranked, rank};
pub use similarity::similarity;
