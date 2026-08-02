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

/// What proving does to the machine it runs on, said before it is too late.
///
/// Proving a defect means writing a test and running the repository's own test
/// suite — three separate runs per defect: a baseline, the full suite after the
/// model's change, and a named-test confirmation. Each is an ordinary process
/// with the permissions of whoever started BugSleuth. A `build.rs`, a procedural
/// macro, a test harness or a cargo config in the reviewed repository executes
/// with those permissions and can read anything that account can read.
///
/// The throwaway worktree is not containment. It stops the reviewed repository
/// being *modified*; it does nothing about what code inside it can reach.
///
/// This is stated rather than fixed because there is no honest way to fix it and
/// still prove anything: proving a defect *is* running its test. What was wrong
/// was leaving it unsaid — the review limits told the reader that code executed
/// and left them to infer the rest, and this tool exists for people who cannot
/// check that inference themselves.
pub const PROVING_EXECUTION_WARNING: &str = "Proving ran this repository's own build and test commands on this machine, with your account's permissions and no sandbox — a build script, a procedural macro or a test fixture in the reviewed code executed as you. The throwaway checkout stops the reviewed code being modified and nothing else. Only ask for proof on a repository you would already be willing to run.";

/// Text from the reviewed repository, made safe to print on a terminal.
///
/// Quoted source is attacker-controlled: the whole premise is that the code
/// under review may be hostile. An ANSI escape sequence inside a snippet is
/// interpreted by the terminal printing the report, not shown — it can move the
/// cursor, clear the screen, recolour text, or overwrite the lines above it. In
/// a tool whose output is a list of security findings, that means a repository
/// can hide the finding about itself.
///
/// Everything below space except tab is replaced with a visible placeholder.
/// Nothing legitimate in a source snippet needs a control character, and seeing
/// `<0x1b>` where one was is more informative than silently dropping it.
#[must_use]
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == '\t' || c == '\n' || !c.is_control() {
                c.to_string()
            } else {
                format!("<0x{:02x}>", c as u32)
            }
        })
        .collect()
}

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

#[cfg(test)]
mod prose_tests {
    /// No message a user reads carries a run of stray spaces.
    ///
    /// Two did: a cancellation notice printed on every cancelled run, and the
    /// triage note rendered verbatim into the report. Both were multi-line
    /// string literals whose continuation lost its `\` at some point, baking the
    /// source's own indentation into the sentence. Nothing objected — a string
    /// is a string — and the one test touching the triage note hand-built the
    /// value and asserted on unrelated substrings, so it reproduced the same
    /// broken spacing rather than catching it.
    #[test]
    fn no_user_facing_string_carries_stray_indentation() {
        let sources = [
            (
                "the cancellation notice",
                include_str!("../../bugsleuth-engine/src/orchestrate/batch.rs"),
            ),
            (
                "the triage note",
                include_str!("../../bugsleuth-engine/src/triage.rs"),
            ),
            (
                "the run report",
                include_str!("../../bugsleuth-engine/src/orchestrate/render.rs"),
            ),
            (
                "the merged report",
                include_str!("../../bugsleuth-engine/src/merge.rs"),
            ),
            ("the review limits", include_str!("limits.rs")),
        ];
        let mut bad: Vec<String> = Vec::new();
        for (name, source) in sources {
            for (number, line) in source.lines().enumerate() {
                // A run of spaces *inside* a line, after a word — not leading
                // indentation, and not the aligned `\` continuations rustfmt
                // itself produces at the end of a line.
                let trimmed = line.trim_start();
                if let Some(at) = trimmed.find("  ")
                    && at > 0
                    && trimmed[..at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == ',')
                    && trimmed[at..]
                        .trim_start()
                        .starts_with(|c: char| c.is_ascii_lowercase())
                    && !trimmed.contains("//")
                {
                    bad.push(format!("{name} line {}: {trimmed}", number + 1));
                }
            }
        }
        assert!(
            bad.is_empty(),
            "these read with a gap in the middle of a sentence:\n{}",
            bad.join("\n")
        );
    }
}
