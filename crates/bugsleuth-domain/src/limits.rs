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
    "No finding here came from running the program. Every one comes from reading \
     code, so defects that only appear when it executes — a race that needs real \
     timing, a leak that needs hours, anything depending on real data or a real \
     network — were not looked for. Proving is the exception: it runs the \
     repository's own test suite in a throwaway checkout, so if you asked for \
     proof, that code did execute on this machine.",
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

/// What cannot be promised about a vendor that has no per-invocation sandbox.
///
/// Claude takes a tool allowlist and Codex takes `--sandbox read-only`, so
/// neither can write or reach the network during a sweep. Kilo has no
/// equivalent: its permissions come from the user's own global configuration,
/// and BugSleuth cannot narrow them for one invocation. The throwaway worktree
/// stops it modifying the repository under review — that is all it stops.
///
/// This matters because the reviewed repository is untrusted input by design.
/// Text in it can address the agent directly, and an agent whose global
/// configuration permits file access and network can be told to read something
/// outside the repository and send it somewhere. Found by BugSleuth reviewing
/// itself, and stated rather than quietly accepted: the user chooses which
/// vendors run, so the user is owed the difference between them.
pub const UNSANDBOXED_VENDOR_WARNING: &str = "This run includes Kilo, which cannot be restricted for a single invocation the way the other vendors can — it runs with whatever permissions your own Kilo configuration grants it. The throwaway checkout stops it modifying the code under review and nothing else. Since the repository being reviewed is untrusted, text inside it can address the agent directly, so only point a Kilo sweep at code you are willing to have an agent with your permissions read.";

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
    fn the_unsandboxed_warning_says_what_is_and_is_not_contained() {
        // The worktree is real containment and easy to over-read. The warning
        // has to say what it does stop and what it does not.
        assert!(UNSANDBOXED_VENDOR_WARNING.contains("throwaway checkout"));
        assert!(UNSANDBOXED_VENDOR_WARNING.contains("nothing else"));
        assert!(UNSANDBOXED_VENDOR_WARNING.contains("untrusted"));
    }

    #[test]
    fn the_list_renders_one_bullet_per_limit() {
        let rendered = as_list("- ");
        assert_eq!(rendered.lines().count(), REVIEW_LIMITS.len());
        assert!(rendered.starts_with("- "));
    }
}
