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
/// **That thin margin was crossed the moment one model was asked to sweep
/// twice.** The measurements above are cross-vendor; two passes of the *same*
/// model word two different defects far more alike, and three unrelated defects
/// in one file scored 0.36 to 0.42 against each other. Wording alone can no
/// longer do this job, which is why [`names_overlap`] now has to agree too.
///
/// Two limitations this leaves in place, both real and neither fixable by a
/// threshold. A compound finding — one model describing a panic *and* an
/// arithmetic error as a single defect — is not one-to-one with either
/// counterpart. And two pairs in the fixture that genuinely are one defect
/// score 0.27 and 0.31, below this, because the two vendors chose almost
/// disjoint vocabulary for the same bug.
pub(crate) const MIN_WORDING_OVERLAP: f32 = 0.35;

pub(super) fn same_defect(left: &Finding, right: &Finding) -> bool {
    // Different lanes ask different questions. A correctness sweep and a
    // security sweep landing on the same lines have found two properties of one
    // piece of code, not one defect twice — and merging keeps only one account,
    // so the reader never learns the other exists. Running more than one lane
    // is the whole reason this tool costs what it costs.
    if left.lane != right.lane {
        return false;
    }
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
    if crate::similarity(&left_text, &right_text) < MIN_WORDING_OVERLAP {
        return false;
    }
    names_overlap(&left_text, &right_text)
}

/// Whether two accounts name any of the same code.
///
/// **The gate that prose could not provide.** Two sweeps of the *same* model
/// word two different defects almost identically — same habits, same sentence
/// shapes — so repeating a model pushed different-defect pairs from around 0.30
/// up past the 0.35 wording threshold. Three unrelated defects in one file
/// collapsed into one on real output: an underflow in `remove_stock`, a
/// division by zero in `average_price`, and an index panic in `top_by_value`,
/// all within ten lines and all scoring 0.36 to 0.42 against each other. No
/// threshold separates that, because the populations now overlap.
///
/// Identifiers do separate it. Two accounts of one defect name the same
/// function; two defects in neighbouring functions name different ones. This
/// keeps the case the line window was widened for — one defect anchored at a
/// signature and at the offending comparison five lines apart, where both
/// accounts still say `snap_all_day_to_midnight`.
///
/// Silent when there is nothing to go on: a finding that names no identifier at
/// all is judged on wording alone, as before. Absent evidence must not read as
/// evidence of difference.
fn names_overlap(left: &str, right: &str) -> bool {
    let (left, right) = (identifiers(left), identifiers(right));
    if left.is_empty() || right.is_empty() {
        return true;
    }
    left.iter().any(|name| right.contains(name))
}

/// Words that look like code rather than prose: `snake_case` or `camelCase`.
/// Deliberately narrow — an ordinary English word appearing in both accounts
/// says nothing, which is exactly the problem this exists to get around.
fn identifiers(text: &str) -> Vec<String> {
    let mut found: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| word.len() > 3 && looks_like_code(word))
        .map(|word| word.to_ascii_lowercase())
        .collect();
    found.sort();
    found.dedup();
    found
}

fn looks_like_code(word: &str) -> bool {
    let inner_underscore = word.trim_matches('_').contains('_');
    let camel = word.chars().next().is_some_and(char::is_lowercase)
        && word.chars().skip(1).any(char::is_uppercase);
    inner_underscore || camel
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
