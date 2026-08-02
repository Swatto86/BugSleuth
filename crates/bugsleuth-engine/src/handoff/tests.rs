//! Tests for the fix-prompt handoff, in their own file only because the module
//! plus its tests crossed the hard line cap.

use super::super::handoff::*;
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
            acknowledged: None,
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

#[test]
fn the_bundle_is_replaced_whole_and_leaves_no_staging_file() {
    // fs::write truncates first, so a failed write left an empty
    // fix-prompt.md where the previous run's entire work order had been.
    let dir = std::env::temp_dir()
        .join("bugsleuth-handoff-atomic")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let entries = vec![ranked_of(1)];
    assert!(write_all(&dir, "C:/repo", &entries, &[], 1).is_ok());
    assert!(write_all(&dir, "C:/repo", &entries, &[], 1).is_ok());

    let names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().any(|n| n == "fix-prompt.md"), "{names:?}");
    assert!(
        !names.iter().any(|n| n.ends_with(".writing")),
        "a staging file was left behind: {names:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
