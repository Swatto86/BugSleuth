//! How alike two findings' wording is.
//!
//! Used only as a *separator*, never as a joiner: two findings must already be
//! anchored to nearby code before wording is consulted at all. The job here is
//! to stop two genuinely different defects on adjacent lines from being merged
//! into one.
//!
//! That situation is not hypothetical. On the fixture used to develop this,
//! Claude reported a divide-by-zero and Codex reported an integer overflow one
//! line apart in the same function. Anchor proximity alone would have merged
//! them and reported one defect where there were two.

/// Words that carry no signal about *which* defect is being described. Every
/// finding says "the", "is", "when"; matching on those inflates every score.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "because", "been", "but", "by", "can", "could",
    "does", "either", "for", "from", "has", "have", "if", "in", "into", "is", "it", "its", "no",
    "not", "of", "on", "or", "so", "than", "that", "the", "then", "there", "this", "to", "up",
    "was", "when", "which", "will", "with", "without", "would",
];

/// How much of the *shorter* description appears in the longer one, 0.0 to 1.0.
///
/// The overlap coefficient, not Jaccard. This started as Jaccard, on the stated
/// grounds that the same defect gets described at very different lengths and
/// "set overlap is not distorted by that the way character-level measures are".
/// That reasoning named exactly the right hazard and then picked the measure
/// that suffers from it: Jaccard divides by the *union*, so a short description
/// fully contained in a long one is bounded by the ratio of their lengths.
///
/// Measured, on real output rather than argued: two vendors reported the same
/// missing-aria-live defect on the same line of the same file, in 22 words and
/// 77 words. Every distinguishing word of the short one appeared in the long
/// one, and Jaccard scored 0.125 — it could not have exceeded 0.29 however
/// perfectly they agreed. Dividing by the smaller set instead scores 0.500.
///
/// The same flaw was already costing merges on the seeded fixture, where two
/// pairs describing one defect scored 0.12 and 0.16, so this was never a
/// property of one awkward repository.
pub fn similarity(left: &str, right: &str) -> f32 {
    let left = significant_words(left);
    let right = significant_words(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let shared = left.iter().filter(|word| right.contains(*word)).count();
    // Divide by the smaller description. Two accounts of one defect agree on
    // that defect's words; the longer one then goes on to say more, and saying
    // more must not look like agreeing less.
    let smaller = left.len().min(right.len());
    shared as f32 / smaller as f32
}

/// Lowercased, de-punctuated, stopword-free, deduplicated words.
///
/// Underscores are split on, not kept: one model writes `average_price` while
/// another writes "the average price", and treating the identifier as one
/// opaque token makes those look unrelated. Splitting it into its parts is what
/// lets two descriptions of the same function match at all — before this, the
/// same defect described by two vendors scored 0.22, below the merge threshold.
///
/// A `Vec` rather than a `HashSet`: these are a handful of words each, and a
/// linear scan over ~20 items beats hashing them. Deduplicated so a word
/// repeated five times in a long explanation does not outweigh the rest.
fn significant_words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.len() < 3 {
            continue;
        }
        let word = stem(&raw.to_lowercase());
        if word.len() < 3 || STOPWORDS.contains(&word.as_str()) || words.contains(&word) {
            continue;
        }
        words.push(word);
    }
    words
}

/// Crude suffix stripping, so grammatical form does not hide a match.
///
/// Not a real stemmer, and deliberately not worth a dependency. It exists
/// because real cross-vendor output showed the same defect described as
/// "remove_stock underflows" by one model and "Removing more stock ...
/// underflows" by another. Without this those shared only three words and the
/// merge threshold was never reached, so the same defect was reported twice.
///
/// Over-stemming is the safe direction here: wording is only ever used to
/// *separate* findings already anchored to the same few lines, so a false match
/// costs a merge of two things that were adjacent anyway, while a missed match
/// costs a duplicate in the report the reader has to reconcile by hand.
fn stem(word: &str) -> String {
    // Longest suffix first: "ies" must be tried before "es" before "s".
    for (suffix, replacement) in [
        ("ies", "y"),
        ("ing", ""),
        ("ted", "t"),
        ("ded", "d"),
        ("ed", ""),
        ("es", ""),
        ("s", ""),
    ] {
        if let Some(root) = word.strip_suffix(suffix)
            // Keep enough of the word that "ties" does not become "y".
            && root.len() >= 3
        {
            return format!("{root}{replacement}");
        }
    }
    word.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The texts below are real output from the cross-vendor run on the seeded
    // fixture, quoted rather than invented, so these tests measure the thing
    // clustering actually has to separate. Clustering compares title *and*
    // explanation, so these do too.

    const DIVIDE_CLAUDE: &str = "average_price divides by zero on an empty inventory. \
        There is no check for an empty inventory before dividing by the item count.";
    const DIVIDE_CODEX: &str = "Calculating the average price of an empty inventory panics. \
        An empty inventory has length zero, so this integer division panics.";
    const OVERFLOW_CODEX: &str = "Average price accumulation can overflow. \
        Summing prices into u64 is unchecked and can exceed the maximum value.";

    /// The score the judge actually merges at. Referenced rather than repeated,
    /// so these tests cannot quietly drift from the threshold they protect —
    /// which is exactly what happened when the measure changed underneath them
    /// and a hardcoded 0.2 turned into a failing assertion about nothing.
    use crate::cluster::pairing::MIN_WORDING_OVERLAP as MERGE_AT;

    /// The pair that exposed the measure, kept verbatim from a real run.
    ///
    /// Two vendors, same file, same line, same missing-aria-live defect — one
    /// in 22 significant words, the other in 77. Under Jaccard this scored
    /// 0.125 and the report carried the defect twice.
    #[test]
    fn one_terse_and_one_thorough_account_of_a_defect_still_merge() {
        let terse = "Status bar updates are invisible to screen readers. \
            The status bar at the bottom of the window is the primary feedback surface for \
            errors, loading, and completion. It has no role or aria-live attribute, so screen \
            readers do not announce any of its text changes.";
        let thorough = "Status bar and inline dialog messages have no aria-live region, screen \
            readers never hear them. `#status` is the single place the app reports the outcome \
            of nearly every action: 'Deleted N messages', 'Message sent.', 'Send cancelled.', \
            and every caught-error path. It is updated purely via `statusEl.textContent` \
            throughout main.ts. A grep across the entire src/ tree for `aria-live`, \
            `role=status`, or `role=alert` returns zero matches - there is no live region \
            anywhere in the app, including the equivalent per-dialog message divs. A screen \
            reader only announces content that either receives focus or lives in an ARIA live \
            region; plain textContent changes to an unfocused div are silent.";

        let score = similarity(terse, thorough);
        assert!(
            score >= MERGE_AT,
            "scored {score}, below the {MERGE_AT} merge threshold — the same defect would be \
             reported twice because one vendor wrote three times as much about it"
        );
    }

    #[test]
    fn a_longer_description_never_scores_worse_for_saying_more() {
        // The property the overlap coefficient buys, stated directly: padding
        // one side with extra detail must not push a real match below the
        // threshold. Under Jaccard it did, which is the whole reason for the
        // change.
        let short = "parse_price panics on a malformed price string";
        let long = "parse_price panics on a malformed price string. It calls unwrap on the \
            result of splitting the input, so any value without a decimal point terminates \
            the process instead of returning an error to the caller.";
        assert!(similarity(short, long) >= similarity(short, short) * 0.5);
        assert!(
            similarity(short, long) >= MERGE_AT,
            "elaboration pushed a genuine match below the threshold"
        );
    }

    #[test]
    fn the_same_defect_described_by_two_vendors_scores_high_enough_to_merge() {
        let score = similarity(DIVIDE_CLAUDE, DIVIDE_CODEX);
        assert!(
            score >= MERGE_AT,
            "scored {score}, below the {MERGE_AT} needed, so the same defect would not merge"
        );
    }

    #[test]
    fn two_different_defects_in_the_same_function_score_far_lower() {
        let same = similarity(DIVIDE_CLAUDE, DIVIDE_CODEX);
        let different = similarity(DIVIDE_CLAUDE, OVERFLOW_CODEX);
        assert!(
            different < MERGE_AT,
            "scored {different}, at or above the {MERGE_AT} merge threshold — \
             these are different defects and must not be merged"
        );
        // The margin is what makes the threshold safe to pick, not the absolute
        // values. If this ratio ever narrows, the measure needs work before the
        // threshold does.
        assert!(
            same > different * 2.0,
            "same-defect {same} vs different-defect {different}: too close to separate reliably"
        );
    }

    #[test]
    fn an_identifier_matches_the_same_name_written_as_words() {
        // `average_price` against "the average price" — one model uses the
        // symbol, another describes it. Without splitting on underscores these
        // scored below the merge threshold and the same defect was reported twice.
        assert!(similarity("average_price is wrong", "the average price is wrong") > 0.9);
    }

    #[test]
    fn identical_text_scores_one_and_unrelated_text_scores_zero() {
        assert_eq!(
            similarity("panic on empty slice", "panic on empty slice"),
            1.0
        );
        assert_eq!(similarity("alpha bravo", "charlie delta"), 0.0);
    }

    #[test]
    fn stopwords_alone_do_not_make_two_findings_look_alike() {
        assert_eq!(similarity("the and with that", "the and with that"), 0.0);
    }

    #[test]
    fn empty_text_never_matches_anything() {
        assert_eq!(similarity("", "anything at all"), 0.0);
        assert_eq!(similarity("", ""), 0.0);
    }

    #[test]
    fn a_repeated_word_does_not_dominate_the_score() {
        let repeated = "overflow overflow overflow overflow quantity";
        let once = "overflow quantity";
        assert_eq!(similarity(repeated, once), 1.0);
    }
}
