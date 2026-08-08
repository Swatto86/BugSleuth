//! Pure types for BugSleuth. No I/O, no async, no dependencies on sibling crates.
//!
//! Everything else in the workspace may depend on this crate; this crate depends
//! on nothing of ours. That one-way rule is what keeps the layering honest.

mod finding;
mod ids;
mod lane;
mod limits;
mod triage;

pub use finding::{
    Finding, FixEdit, FixPlan, RawFinding, RawFindings, Severity, VerifiedAnchor, finding_schema,
};
pub use ids::{FindingId, LaneId, ModelId, RunId};
pub use lane::Lane;
pub use limits::{REVIEW_LIMITS, UNSANDBOXED_VENDOR_WARNING, as_list as limits_list, printable};
pub use triage::{SEVERITY_RUBRIC, SeverityVerdict, SeverityVerdicts, triage_schema};
