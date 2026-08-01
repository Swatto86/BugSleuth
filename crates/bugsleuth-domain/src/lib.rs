//! Pure types for BugSleuth. No I/O, no async, no dependencies on sibling crates.
//!
//! Everything else in the workspace may depend on this crate; this crate depends
//! on nothing of ours. That one-way rule is what keeps the layering honest.

mod finding;
mod ids;
mod lane;
mod limits;
mod proof;
mod triage;

pub use finding::{
    Finding, FixEdit, FixPlan, RawFinding, RawFindings, Severity, VerifiedAnchor, finding_schema,
};
pub use ids::{FindingId, LaneId, ModelId, RunId};
pub use lane::Lane;
pub use limits::{REVIEW_LIMITS, as_list as limits_list};
pub use proof::{ProofClaim, ProofVerdict, proof_schema};
pub use triage::{SEVERITY_RUBRIC, SeverityVerdict, SeverityVerdicts, triage_schema};
