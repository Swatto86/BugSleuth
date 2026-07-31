//! Pure types for BugSleuth. No I/O, no async, no dependencies on sibling crates.
//!
//! Everything else in the workspace may depend on this crate; this crate depends
//! on nothing of ours. That one-way rule is what keeps the layering honest.

mod finding;
mod ids;
mod lane;
mod proof;

pub use finding::{Finding, RawFinding, RawFindings, Severity, VerifiedAnchor, finding_schema};
pub use ids::{FindingId, LaneId, ModelId, RunId};
pub use lane::{Lane, LaneScope};
pub use proof::{ProofClaim, ProofVerdict, proof_schema};
