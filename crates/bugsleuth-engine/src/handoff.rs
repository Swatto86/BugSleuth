//! The part of the report an implementer actually works from.
//!
//! The ranked list answers "what is wrong and how bad". This answers "what do I
//! do about it", and it is written for a specific reader: a model running
//! locally on the user's own machine, given the repository and this text and
//! nothing else. It has not read the review. It cannot ask a question. It may
//! be a good deal weaker than whichever model found the defect.
//!
//! So the handoff repeats things the summary already said. That is deliberate —
//! each defect is a self-contained work order, because an implementer told to
//! fix defect 7 should not have to reconstruct defects 1 to 6 to understand it.

use bugsleuth_judge::Ranked;

mod work_order;
pub use work_order::work_order;

/// Assemble the whole prompt: instructions, what was skipped, then the work.
///
/// Takes the pieces rather than a report type, because two different reports
/// carry them — a full `run` and a standalone `judge` — and the prompt handed to
/// an agent must be identical either way.
///
/// `not_reviewed` is not optional politeness. An agent given a list of defects
/// will assume it is the whole list unless it is told which parts of the
/// repository nobody looked at.
#[must_use]
pub fn prompt(repo: &str, ranked: &[Ranked], not_reviewed: &[String], sweeps: usize) -> String {
    let mut out = String::new();

    if !not_reviewed.is_empty() {
        out.push_str("\n## What was NOT reviewed\n\n");
        for gap in not_reviewed {
            out.push_str(&format!("- {}\n", short_reason(gap)));
        }
        out.push_str(
            "\nNothing was looked for in those. Their absence from the list below \
             is not evidence that they are clean.\n",
        );
    }

    // The gaps that remain even when every lane ran. Without these, a list of
    // three defects reads as "the three problems" rather than "three of the
    // problems a reader of source can find" — and the agent acting on it draws
    // the same wrong conclusion the human would.
    out.push_str(&crate::caveats::limits(""));

    out.push_str(&format!(
        "\n## The defects, worst first ({})\n",
        ranked.len()
    ));
    for entry in ranked {
        out.push_str(&work_order(
            entry.position,
            entry.cluster.representative(),
            // The cluster's severity, not the finder's own: after a triage pass
            // they differ, and a prompt that disagreed with the report it came
            // from would have someone fixing defect 7 while reading it graded
            // two bands apart.
            entry.cluster.severity(),
            entry.cluster.agreement,
            sweeps,
        ));
    }

    let mut full = preamble(repo);
    full.push_str(&size_note(&out, ranked.len()));
    full.push_str(&out);
    full
}

/// A rough token count, and a warning about the failure it invites.
///
/// A local model given more than its context can hold does not refuse — it
/// truncates and answers confidently about the part it received. Measured on a
/// real repository this prompt reached ~23,700 tokens for 23 defects, which
/// overruns a 32k model once file reads and output are added, and is cut to
/// nothing useful by a runtime left at its 4,096 default. That failure is
/// invisible from both ends, so the document states its own size and gives a
/// check the reader can actually perform.
///
/// Characters over four is the usual rough ratio. It is approximate and says
/// so: an estimate that prompts someone to check beats no estimate at all.
fn size_note(body: &str, defects: usize) -> String {
    let tokens = body.chars().count() / 4;
    format!(
        "\n## Before you start: is this whole document in your context?\n\n\
         This prompt describes **{defects} defects** and is roughly \
         **{tokens} tokens**.\n\n\
         If your context window cannot hold all of it, you will not be told — \
         you will simply receive the beginning and answer about that, which \
         looks identical to answering about everything. So check: if you cannot \
         see a heading numbered `{defects}.` near the end of this document, you \
         have been truncated. Say so and stop.\n\n\
         Each defect is also available as its own file, `fix-prompt-01.md` \
         onward, next to this one. Those are self-contained and small. Use them \
         rather than this if the whole thing does not fit.\n"
    )
}

/// One defect on its own, as a complete standalone prompt.
///
/// The same work order with the same instructions around it, so a small model
/// can be handed one defect at a time without losing the scepticism or the
/// requirement to leave a test behind — which is also the workflow the bundle
/// asks for, made structural rather than merely instructed.
#[must_use]
pub fn single(repo: &str, entry: &Ranked, total: usize, sweeps: usize) -> String {
    let mut out = preamble(repo);
    out.push_str(&format!(
        "\n## This is defect {} of {}\n\n\
         The others are in the files beside this one. Fix only this one, then \
         stop and report.\n",
        entry.position, total
    ));
    out.push_str(&work_order(
        entry.position,
        entry.cluster.representative(),
        entry.cluster.severity(),
        entry.cluster.agreement,
        sweeps,
    ));
    out
}

/// Write the bundle and one file per defect into `dir`.
///
/// Returns the bundle's path. Per-defect files are best-effort: failing to
/// write one is not a reason to lose the bundle, which is what most people use.
pub fn write_all(
    dir: &std::path::Path,
    repo: &str,
    ranked: &[Ranked],
    not_reviewed: &[String],
    sweeps: usize,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;

    // Written first, deleted after. This used to clear the previous run's
    // per-defect files *before* writing anything, so a bundle write that failed
    // — a full disk, a permission change — left the last run's work orders
    // deleted and the new ones never created, losing tens of minutes of paid
    // sweeping to a failure that had nothing to do with them.
    //
    // Found by BugSleuth reviewing itself, by both vendors, and it is exactly
    // the destroy-before-commit class the correctness mandate was taught to
    // hunt for the day before.
    let bundle = dir.join("fix-prompt.md");
    std::fs::write(&bundle, prompt(repo, ranked, not_reviewed, sweeps))?;

    let mut written: Vec<String> = Vec::new();
    for entry in ranked {
        let name = format!("fix-prompt-{:02}.md", entry.position);
        if std::fs::write(dir.join(&name), single(repo, entry, ranked.len(), sweeps)).is_ok() {
            written.push(name.to_lowercase());
        }
    }

    // Only now are the leftovers from a previous, longer run removed — a file
    // this run did not just write is one that would otherwise be read as part
    // of this report.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let stale = name.starts_with("fix-prompt-")
                && name.ends_with(".md")
                && !written.contains(&name);
            if stale {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(bundle)
}

/// The opening of the prompt, addressed to whoever is going to do the work.
///
/// Written as an instruction to a model, not as a description of a report,
/// because that is how it gets used: copied whole and pasted into a coding
/// agent pointed at the repository.
///
/// The scepticism is deliberate and is backed by measurement. Every location
/// here was checked to exist and the quoted code was found in it, but the
/// *reasoning* about that code was never executed. On a graded experiment
/// against known defects, one reviewer reported a fault in the same file and
/// the same function as a real bug, and it was a different bug — plausible,
/// well-argued and wrong. An implementer that treats this document as fact will
/// eventually "fix" something that was never broken.
pub fn preamble(repo: &str) -> String {
    format!(
        "You are fixing defects in the repository at `{repo}`.\n\n\
         Below is a list of defects found by an automated review. Each is a \
         self-contained work order: where it is, the code as it stands, why it \
         is wrong, how it fails, and what to change. Work them in the order \
         given — they are ordered by severity.\n\n\
         **Treat each one as a claim to verify, not a fact.** Every location was \
         checked to exist and the quoted code really is in that file, but no \
         reviewer ran the code. Some of these will be wrong in ways that read \
         convincingly.\n\n\
         For each defect, in order:\n\n\
         1. **Read the code first.** If it does not do what the finding says, \
         stop and say so. Do not edit it into agreement with the finding.\n\
         2. **Fix the root cause, not the line.** Where a fix names other \
         callers, change them too — a guard added to one caller and not its \
         siblings leaves the defect in.\n\
         3. **Leave the check behind.** Add the test described under \"Prove it \
         is fixed\" and run it. It must fail before your change and pass after. \
         A fix with no such test is a claim, not a fix.\n\
         4. **One defect per commit.** Do not roll several together; if one \
         turns out to be wrong, the rest should not be reverted with it.\n\n\
         When you are done, report for each defect: fixed, not a real defect \
         (with why), or could not fix (with what blocked you). Do not silently \
         skip any.\n"
    )
}

/// A gap line with the raw model output cut off it.
///
/// Failure reasons quote what the model actually said, which is the right thing
/// in the operator's report and the wrong thing here. One real reason carried a
/// truncated half-finished *finding* from a malformed reply — and this document
/// is read by an agent that is being handed findings to act on. Text that looks
/// like an instruction must not arrive by accident.
///
/// The operator still sees the whole reason; only the prompt is trimmed.
fn short_reason(gap: &str) -> String {
    const CUTS: [&str; 3] = ["; reply began", "; it began", "; the reply began"];
    let mut trimmed = gap;
    for cut in CUTS {
        if let Some(at) = trimmed.find(cut) {
            trimmed = &trimmed[..at];
        }
    }
    // A backstop for reasons that quote without one of those markers.
    const MAX: usize = 160;
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let short: String = trimmed.chars().take(MAX).collect();
    format!("{short}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::{Finding, FindingId, LaneId, ModelId, Severity, VerifiedAnchor};

    fn finding(fix: bugsleuth_domain::FixPlan) -> Finding {
        Finding {
            id: FindingId::new("f1"),
            lane: LaneId::new("Correctness"),
            model: ModelId::new("claude:sonnet"),
            title: "Divides by zero on an empty inventory".into(),
            severity: Severity::High,
            anchor: VerifiedAnchor {
                file: "src/price.rs".into(),
                line: 42,
                claimed_line: 40,
                snippet: "let avg = total / items.len();".into(),
            },
            explanation: "No check for an empty inventory.".into(),
            failure_scenario: "An empty inventory panics.".into(),
            fix,
        }
    }

    #[test]
    fn a_gap_line_does_not_smuggle_another_models_findings_into_the_prompt() {
        // Real reason from a run. Kilo answered with malformed JSON, and the
        // failure reason quoted it — including a half-written finding. This
        // document is handed to an agent that acts on findings, so text that
        // reads like one must not arrive by accident.
        let gap = "Correctness lane, by kilo — the model's reply did not match the required                    structure: missing field `title`; reply began \"{\\\"findings\\\":                   [{\\\"category\\\":\\\"correctness\\\",\\\"description\\\":                   \\\"All-day event window overlap is checked with lexicographic string";
        let line = short_reason(gap);
        assert!(
            !line.contains("findings"),
            "raw model output survived: {line}"
        );
        assert!(!line.contains("All-day event"), "a finding leaked: {line}");
        assert!(
            line.contains("missing field"),
            "the useful part was lost: {line}"
        );
    }

    #[test]
    fn a_gap_with_no_quoted_output_is_left_alone() {
        let gap = "Security lane, by claude:sonnet — the claude CLI exited with code 1";
        assert_eq!(short_reason(gap), gap);
    }

    #[test]
    fn the_bundle_states_its_own_size_and_how_to_avoid_being_truncated() {
        // The failure this guards against is invisible from both ends: a model
        // given more than its context holds answers confidently about the part
        // it received, and the reader cannot tell the difference.
        let ranked = vec![ranked_of(1), ranked_of(2)];
        let text = prompt("C:/x", &ranked, &[], 3);
        assert!(
            text.contains("**2 defects**"),
            "the defect count is not stated"
        );
        assert!(text.contains("tokens"), "no size estimate");
        assert!(
            text.contains("you will not be told"),
            "truncation is not warned about"
        );
        assert!(
            text.contains("fix-prompt-01.md"),
            "the fallback is not offered"
        );
    }

    #[test]
    fn a_single_defect_prompt_stands_on_its_own() {
        // Handed to a small model alone it must still carry the scepticism and
        // the leave-a-test rule, or the per-defect route is a downgrade.
        let entry = ranked_of(2);
        let text = single("C:/x", &entry, 5, 3);
        assert!(text.contains("defect 2 of 5"));
        assert!(text.contains("claim to verify"), "lost the scepticism");
        assert!(text.contains("fail before"), "lost the test requirement");
        assert!(text.contains("Divides by zero"), "lost the defect itself");
    }

    #[test]
    fn writing_prompts_clears_a_previous_runs_per_defect_files() {
        // A shorter run leaving fix-prompt-09.md behind would have it read as
        // part of this report.
        let dir = std::env::temp_dir()
            .join("bugsleuth-writeall-tests")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.join("fix-prompt-09.md"), "stale").unwrap_or_else(|e| panic!("{e}"));

        write_all(&dir, "C:/x", &[ranked_of(1)], &[], 1).unwrap_or_else(|e| panic!("{e}"));

        assert!(dir.join("fix-prompt.md").exists());
        assert!(dir.join("fix-prompt-01.md").exists());
        assert!(
            !dir.join("fix-prompt-09.md").exists(),
            "a per-defect prompt from an earlier run survived"
        );
    }

    fn ranked_of(position: usize) -> Ranked {
        Ranked {
            position,
            cluster: bugsleuth_judge::Cluster {
                findings: vec![finding(Default::default())],
                agreement: 1,
                triaged: None,
                triage_reason: None,
            },
        }
    }

    #[test]
    fn the_fix_prompt_tells_the_agent_what_the_review_could_not_see() {
        // The agent draws the same wrong conclusion the human would: a list of
        // three defects with no stated blind spots reads as the whole problem.
        let text = prompt("C:/repo", &[ranked_of(1)], &[], 2);
        assert!(text.contains("could not see"), "{text}");
        assert!(text.contains("Only code inside this repository"), "{text}");
    }
}
