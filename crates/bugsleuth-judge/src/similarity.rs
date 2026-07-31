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

/// Jaccard overlap of the significant words in two texts, from 0.0 to 1.0.
///
/// Jaccard rather than a substring or edit distance because the same defect
/// gets described at very different lengths — "divides by zero on an empty
/// inventory" against a three-sentence explanation — and set overlap is not
/// distorted by that the way character-level measures are.
pub fn similarity(left: &str, right: &str) -> f32 {
    let left = significant_words(left);
    let right = significant_words(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let shared = left.iter().filter(|word| right.contains(*word)).count();
    let total = left.len() + right.len() - shared;
    if total == 0 {
        return 0.0;
    }
    shared as f32 / total as f32
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

    #[test]
    fn the_same_defect_described_by_two_vendors_scores_high_enough_to_merge() {
        let score = similarity(DIVIDE_CLAUDE, DIVIDE_CODEX);
        assert!(
            score > 0.3,
            "scored {score}, so the same defect would not merge"
        );
    }

    #[test]
    fn two_different_defects_in_the_same_function_score_far_lower() {
        let same = similarity(DIVIDE_CLAUDE, DIVIDE_CODEX);
        let different = similarity(DIVIDE_CLAUDE, OVERFLOW_CODEX);
        assert!(
            different < 0.2,
            "scored {different} — these must not be merged"
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
