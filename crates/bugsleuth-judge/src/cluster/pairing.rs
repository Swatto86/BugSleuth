//! Deciding whether two findings describe the same defect.
//!
//! Kept apart from the clustering that uses it because these are the two
//! measured thresholds in the project, and the evidence behind each is longer
//! than the code it governs. Burying them inside the grouping loop hid the fact
//! that the thresholds, not the algorithm, are what decides the answer.

use bugsleuth_domain::{Finding, Severity};

/// How far apart two anchors may be and still be candidates for the same defect.
///
/// **Widened from 3 after real output showed 3 was too tight.** Reviewing a real
/// repository, two vendors independently found the same bug in one small
/// function — a timezone heuristic that assumes offsets are under 12 hours — but
/// anchored it 5 lines apart, one at the signature and one at the offending
/// comparison. They scored 0.25 on wording, well above the merge threshold, so
/// only the distance kept them apart and the report claimed two defects with no
/// agreement instead of one found by both.
///
/// Roughly "within the same small function". Widening this is safe because
/// wording is the real separator: on measured output, different defects on
/// *adjacent* lines score 0.07-0.08 against a 0.20 threshold, so distance is not
/// what is doing the work.
const MAX_LINE_GAP: u32 = 10;

/// Minimum wording overlap for two nearby findings to be treated as one.
///
/// **Measured, not guessed**, and re-measured when the measure itself changed.
/// Every same-file, within-ten-line, cross-vendor pair from three real corpora —
/// the seeded fixture, a real crate, and a full Alder run — was scored by
/// hand. Under the overlap coefficient:
///
/// | | range |
/// |---|---|
/// | pairs describing the same defect | 0.39 – 0.65 |
/// | pairs describing different defects in the same function | 0.06 – 0.33 |
///
/// 0.35 sits in that gap. The margin is thin — 0.02 on the wrong-merge side —
/// and that is worth saying plainly rather than dressing up: the two
/// populations nearly touch, because two defects in one function share its
/// vocabulary almost as much as two accounts of one defect do.
///
/// Erring toward *not* merging remains the right direction: a duplicate in the
/// report is an annoyance, while two distinct defects silently collapsed into
/// one means the reader never learns about the second.
///
/// Two limitations this leaves in place, both real and neither fixable by a
/// threshold. A compound finding — one model describing a panic *and* an
/// arithmetic error as a single defect — is not one-to-one with either
/// counterpart. And two pairs in the fixture that genuinely are one defect
/// score 0.27 and 0.31, below this, because the two vendors chose almost
/// disjoint vocabulary for the same bug.
pub(crate) const MIN_WORDING_OVERLAP: f32 = 0.35;

pub(super) fn same_defect(left: &Finding, right: &Finding) -> bool {
    if left.anchor.file != right.anchor.file {
        return false;
    }
    if left.anchor.line.abs_diff(right.anchor.line) > MAX_LINE_GAP {
        return false;
    }
    // Compare title plus explanation. The title alone is often too short to
    // separate two defects in the same function; the explanation carries the
    // words that distinguish "divides by zero" from "can overflow".
    let left_text = format!("{} {}", left.title, left.explanation);
    let right_text = format!("{} {}", right.title, right.explanation);
    crate::similarity(&left_text, &right_text) >= MIN_WORDING_OVERLAP
}

/// Whether a finding carries usable fix instructions.
pub(super) fn has_plan(finding: &Finding) -> bool {
    !finding.fix.approach.trim().is_empty() || !finding.fix.edits.is_empty()
}

pub(crate) fn severity_order(severity: Severity) -> u8 {
    match severity {
        Severity::Critical => 0,
        Severity::High => 1,
        Severity::Medium => 2,
        Severity::Low => 3,
    }
}
