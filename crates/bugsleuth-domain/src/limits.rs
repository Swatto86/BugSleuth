//! What a BugSleuth review cannot see, whatever it finds.
//!
//! The report already names every lane nobody swept, because a silent gap reads
//! as a clean bill of health. These are the gaps that remain when *every* lane
//! ran: limits of the method rather than of the configuration, and no amount of
//! adding models closes any of them.
//!
//! They are worth stating for the same reason the unswept lanes are. A reader
//! who cannot check the code is handed a list of three defects; nothing on the
//! page tells them whether that is "three problems" or "three of the problems a
//! reader of source can find". Two of these were learned rather than reasoned:
//! a real contract defect in a real repository turned out to be a disagreement
//! with a remote server that no reader of the code could have seen, and the
//! security lane's "dependency risk" was never a check against any published
//! vulnerability.

/// The limits, worst-misunderstood first.
///
/// Deliberately a fixed list and not per-lane. Every one of these describes how
/// the review is *conducted* — reading code, offline, without running anything —
/// so every one applies to every lane. A per-lane version would be more precise
/// and would mean nobody read it.
pub const REVIEW_LIMITS: [&str; 5] = [
    "Nothing was run. Every finding comes from reading code, so defects that \
     only appear when the program executes — a race that needs real timing, a \
     leak that needs hours, anything that depends on real data or a real \
     network — were not looked for.",
    "Only code inside this repository was read. Where the code has to agree \
     with something outside it — a remote API's requirements, another service's \
     actual behaviour, the operating system's — only one side of that agreement \
     was visible, so a disagreement cannot be found here.",
    "Dependencies were not checked against any vulnerability database. The \
     security lane covers what this code does with a dependency; it says \
     nothing about whether the version pinned has a published advisory.",
    "Nothing looked at the running application. The interface lane reads code \
     for behavioural defects — an action with no keyboard path, a failure shown \
     to nobody — and cannot see what the app actually draws.",
    "A lane that reported nothing means nothing was found, not that nothing is \
     there. Recall has been measured at 3 of 3 on one small set of known \
     defects, which is encouraging and is not a guarantee about yours.",
];

/// The limits as a markdown list, for the fix prompt and the report.
pub fn as_list(bullet: &str) -> String {
    REVIEW_LIMITS
        .iter()
        .map(|limit| format!("{bullet}{limit}\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_limit_is_a_statement_about_method_rather_than_a_hedge() {
        // A limit that does not say what was *not done* is just a disclaimer,
        // and a disclaimer trains the reader to skip the section.
        for limit in REVIEW_LIMITS {
            assert!(
                limit.contains("not")
                    || limit.contains("cannot")
                    || limit.contains("Nothing")
                    || limit.contains("only"),
                "this limit states no absence: {limit}"
            );
            assert!(limit.len() > 80, "too vague to act on: {limit}");
        }
    }

    #[test]
    fn the_list_renders_one_bullet_per_limit() {
        let rendered = as_list("- ");
        assert_eq!(rendered.lines().count(), REVIEW_LIMITS.len());
        assert!(rendered.starts_with("- "));
    }
}
