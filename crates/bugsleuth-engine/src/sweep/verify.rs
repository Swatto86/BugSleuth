//! Splitting a sweep's raw findings into verified and rejected.
//!
//! Extracted from the sweep module at the hard line cap, along the seam that was
//! already there: this is pure anchor verification over what a model claimed,
//! with no knowledge of which vendor produced the findings or how the sweep ran.

use std::path::Path;

use bugsleuth_domain::{Finding, FindingId, Lane, ModelId, RawFinding};
use bugsleuth_verify::verify_anchor;

use crate::report::{Rejected, rank};

/// Split reported findings into those whose quoted code was located in the file
/// they name, and those that were not.
pub(super) fn verify_all(
    repo: &Path,
    lane: Lane,
    model: &ModelId,
    raw: Vec<RawFinding>,
) -> (Vec<Finding>, Vec<Rejected>) {
    let mut verified = Vec::new();
    let mut rejected = Vec::new();

    for (index, finding) in raw.into_iter().enumerate() {
        match verify_anchor(repo, &finding) {
            Ok(anchor) => {
                let id = FindingId::new(format!("{}-{index}", lane.slug()));
                verified.push(Finding::new(id, lane, model.clone(), finding, anchor));
            }
            Err(reason) => rejected.push(Rejected {
                title: finding.title,
                claimed_file: finding.file,
                claimed_line: finding.line,
                reason: reason.to_string(),
            }),
        }
    }

    rank(&mut verified);
    (verified, rejected)
}
