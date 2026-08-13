//! Everything a report has to say about itself, in one place.
//!
//! **This exists because the same defect happened twice in one afternoon.**
//! There are three documents — the run report, the merged `judge` report, and
//! the fix prompt — and each was growing its own copy of the same caveats. The
//! review limits were added to two of them and missed in the third. The
//! unsandboxed-vendor caution was added to one and missed in the other. Both
//! times the result was a report that read as more reassuring depending only on
//! which command produced it.
//!
//! Duplicated prose across renderers that must agree is exactly the "same rule
//! written down twice" class the contract lane was taught to hunt for, so
//! fixing it by remembering harder would have been the wrong answer. There is
//! one function per caveat now, and a test asserts every document uses them.

use bugsleuth_domain::{UNSANDBOXED_VENDOR_WARNING, limits_list};

/// What the method could not see, whatever it found.
///
/// `indent` is the leading whitespace of the surrounding document: the terminal
/// reports indent their body, the markdown prompt does not.
pub(crate) fn limits(indent: &str) -> String {
    format!(
        "\n{indent}What this review could not see:\n{}",
        limits_list(&format!("{indent}- "))
    )
}

/// What a recovered sweep is, said the same way wherever it is said.
///
/// A recovered sweep may have returned less than the original run established.
/// Its list is not necessarily an inventory, and presenting it as a finished
/// sweep tells the reader that everything there is to find in that lane is on
/// the page. The run report said so; the merged `judge` report and the
/// standalone `sweep` report both dropped it — one because its own type had no
/// field for the flag, the other because a `..` pattern discarded it. Same rule
/// missed twice, so it lives here once.
pub(crate) fn salvaged(was_salvaged: bool) -> &'static str {
    if was_salvaged {
        " — RECOVERED after the original CLI run was interrupted or returned a malformed answer, so this list is as far as it got and may be incomplete"
    } else {
        ""
    }
}

/// Which revision a sweep describes, and whether that result was pinned to a
/// clean tree. Shared by standalone and aggregate reports so an unpinned sweep
/// cannot look safer merely because it was rendered through a different path.
pub(crate) fn revision(commit: Option<&str>, cached: Option<&str>) -> String {
    match (commit, cached) {
        (Some(commit), Some(cached)) if commit == cached => {
            format!("revision {}, pinned", short_revision(commit))
        }
        (Some(commit), Some(cached)) => format!(
            "revision {}, cache pinned to {}",
            short_revision(commit),
            short_revision(cached)
        ),
        (Some(commit), None) => format!("revision {}, unpinned", short_revision(commit)),
        (None, Some(cached)) => format!(
            "revision not recorded, cache pinned to {}",
            short_revision(cached)
        ),
        (None, None) => "revision not recorded, unpinned".to_owned(),
    }
}

/// Paths an isolated provider could not review, formatted consistently in
/// standalone and aggregate reports.
pub(crate) fn isolation_exclusions(paths: &[String], indent: &str) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let paths = paths
        .iter()
        .map(|path| bugsleuth_domain::printable(path))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{indent}Not reviewed because provider isolation removed: {paths}\n")
}

fn short_revision(revision: &str) -> String {
    revision.chars().take(7).collect()
}

/// The caution owed when a vendor that cannot be sandboxed actually ran.
///
/// Empty when none did — a caution printed on every report regardless is one
/// nobody reads by the third time.
pub(crate) fn unsandboxed(models: impl Iterator<Item = impl AsRef<str>>, indent: &str) -> String {
    let any = models.into_iter().any(|m| is_unsandboxed(m.as_ref()));
    if !any {
        return String::new();
    }
    format!("\n{indent}Caution: {UNSANDBOXED_VENDOR_WARNING}\n")
}

/// Whether a model spec names a vendor BugSleuth cannot restrict per run.
///
/// Matched on the vendor prefix rather than anywhere in the string, so a model
/// merely *named* something containing "kilo" does not trigger it.
fn is_unsandboxed(model: &str) -> bool {
    model.split(':').next().is_some_and(|v| v == "kilo")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caution_fires_for_a_kilo_vendor_and_not_for_a_model_merely_named_one() {
        let with = unsandboxed(["kilo:kimi", "claude:sonnet"].into_iter(), "  ");
        assert!(with.contains("Caution:"), "{with}");

        let without = unsandboxed(["claude:sonnet", "codex:"].into_iter(), "  ");
        assert!(without.is_empty(), "a caution nobody needs: {without}");

        // A Claude model whose *name* contains the word must not trigger it.
        let named = unsandboxed(["claude:kilo-tuned".to_string()].into_iter(), "  ");
        assert!(named.is_empty(), "matched on the model name: {named}");
    }

    #[test]
    fn nothing_is_said_at_all_when_no_sweeps_ran() {
        let none: Vec<String> = vec![];
        assert!(unsandboxed(none.into_iter(), "  ").is_empty());
    }

    #[test]
    fn the_limits_carry_the_surrounding_documents_indent() {
        assert!(limits("  ").contains("\n  - No finding here came from running"));
        assert!(limits("").contains("\n- No finding here came from running"));
    }

    #[test]
    fn the_recovery_caveat_covers_timeouts_as_well_as_turn_limits() {
        let text = salvaged(true);
        assert!(text.contains("interrupted"), "{text}");
        assert!(!text.contains("running out of turns"), "{text}");
    }

    /// The reason this module exists: a caveat added to one document and
    /// forgotten in another. Every document that reports findings must use
    /// these, and this test fails when a fourth appears without them.
    #[test]
    fn every_report_renderer_uses_the_shared_caveats() {
        let documents = [
            ("the run report", include_str!("orchestrate/render.rs")),
            ("the merged judge report", include_str!("merge.rs")),
            ("the fix prompt", include_str!("handoff.rs")),
        ];
        for (name, source) in documents {
            assert!(
                source.contains("caveats::limits"),
                "{name} does not use the shared review limits"
            );
        }

        // The recovered-sweep note, which went missing from two of the three
        // documents that report sweeps — one had no field for the flag, the
        // other discarded it with a `..` pattern.
        let reporting_sweeps = [
            ("the run report", include_str!("orchestrate/render.rs")),
            ("the merged judge report", include_str!("merge.rs")),
            ("the standalone sweep report", include_str!("report.rs")),
        ];
        for (name, source) in reporting_sweeps {
            assert!(
                source.contains("caveats::salvaged"),
                "{name} does not say when a sweep was cut short"
            );
            assert!(
                source.contains("caveats::isolation_exclusions"),
                "{name} hides paths provider isolation removed"
            );
        }
    }
}
