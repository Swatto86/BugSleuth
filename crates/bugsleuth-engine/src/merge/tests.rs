//! Tests for merging sweep reports, split out at the hard line cap.

use super::super::merge::*;

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&path, body);
    path
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("bugsleuth-merge-tests")
        .join(format!("{}-{name}", std::process::id()))
}

const SWEPT: &str = r#"{
        "lane":"Correctness","model":"claude:sonnet",
        "status":{"state":"swept","turns":8},
        "findings":[{
            "id":"c-0","lane":"correctness","model":"claude:sonnet",
            "title":"average_price divides by zero on an empty inventory",
            "severity":"high",
            "anchor":{"file":"src/inventory.rs","line":43,"claimed_line":43,"snippet":"total / len"},
            "explanation":"No check for an empty inventory before dividing by the item count.",
            "failure_scenario":"f"}],
        "rejected":[]
    }"#;

const SWEPT_OTHER: &str = r#"{
        "lane":"Correctness","model":"codex:",
        "status":{"state":"swept"},
        "findings":[{
            "id":"x-0","lane":"correctness","model":"codex:",
            "title":"Calculating the average price of an empty inventory panics",
            "severity":"medium",
            "anchor":{"file":"src/inventory.rs","line":43,"claimed_line":43,"snippet":"total / len"},
            "explanation":"An empty inventory has length zero, so this integer division panics.",
            "failure_scenario":"f"}],
        "rejected":[]
    }"#;

const FAILED: &str = r#"{
        "lane":"Security","model":"codex:",
        "status":{"state":"not_swept","reason":"the codex CLI exited with code 1"},
        "findings":[],"rejected":[]
    }"#;

#[test]
fn the_same_defect_from_two_vendors_merges_and_records_agreement() {
    let dir = scratch("merge-two");
    let paths = vec![
        write(&dir, "a.json", SWEPT),
        write(&dir, "b.json", SWEPT_OTHER),
    ];
    let merged = merge(&paths).unwrap_or_else(|e| panic!("merge failed: {e}"));
    assert_eq!(merged.ranked.len(), 1, "the same defect was not merged");
    assert_eq!(merged.ranked[0].cluster.agreement, 2);
    // Severity is normalised upward: one said high, one said medium.
    assert_eq!(
        merged.ranked[0].cluster.severity().as_str(),
        "high",
        "a cluster must not be presented more mildly than its worst assessment"
    );
}

#[test]
fn a_failed_sweep_is_reported_as_unswept_and_never_counted_as_clean() {
    let dir = scratch("merge-failed");
    let paths = vec![write(&dir, "a.json", SWEPT), write(&dir, "f.json", FAILED)];
    let merged = merge(&paths).unwrap_or_else(|e| panic!("merge failed: {e}"));
    assert_eq!(merged.unswept.len(), 1);
    assert_eq!(merged.sources.len(), 1);

    let text = merged.to_text();
    assert!(text.contains("NOT SWEPT"));
    assert!(text.contains("Security"));
    assert!(text.contains("NOT reviewed"));
}

#[test]
fn an_unreadable_report_is_an_error_rather_than_a_silently_empty_merge() {
    let dir = scratch("merge-bad");
    let paths = vec![write(&dir, "bad.json", "{ not json")];
    assert!(merge(&paths).is_err());
}

#[test]
fn sweeps_of_two_different_commits_are_called_out_in_the_merged_report() {
    // A set of correct findings was once re-graded against a different
    // checkout and condemned as fabricated - the cited code genuinely was
    // not there. Nothing in the report said which tree each sweep saw.
    let dir = scratch_dir("mixed-commits");
    let a = dir.join("a.json");
    let b = dir.join("b.json");
    let _ = std::fs::write(
        &a,
        r#"{"lane":"UX","model":"claude:sonnet","commit":"aaaaaaaaaaaaaaaa","status":{"state":"swept"},"findings":[]}"#,
    );
    let _ = std::fs::write(
        &b,
        r#"{"lane":"UX","model":"codex:","commit":"bbbbbbbbbbbbbbbb","status":{"state":"swept"},"findings":[]}"#,
    );
    let merged = merge(&[a, b]).unwrap_or_else(|e| panic!("merge failed: {e}"));
    assert_eq!(merged.commits.len(), 2);
    assert!(merged.to_text().contains("WARNING"), "{}", merged.to_text());
    assert!(merged.to_text().contains("2 different commits"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sweeps_of_one_commit_and_reports_with_none_recorded_stay_quiet() {
    // Every report written before commits were recorded has none; loading
    // them must neither fail nor warn.
    let dir = scratch_dir("one-commit");
    let a = dir.join("a.json");
    let b = dir.join("b.json");
    let _ = std::fs::write(
        &a,
        r#"{"lane":"UX","model":"claude:sonnet","commit":"aaaaaaaaaaaaaaaa","status":{"state":"swept"},"findings":[]}"#,
    );
    let _ = std::fs::write(
        &b,
        r#"{"lane":"UX","model":"codex:","status":{"state":"swept"},"findings":[]}"#,
    );
    let merged = merge(&[a, b]).unwrap_or_else(|e| panic!("merge failed: {e}"));
    assert_eq!(merged.commits.len(), 1);
    assert!(
        !merged.to_text().contains("WARNING"),
        "{}",
        merged.to_text()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-merge-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[test]
fn the_merged_report_states_the_methods_blind_spots_too() {
    // Both renderers reach someone deciding whether the code is in good
    // shape, so both have to say what was structurally not looked for. This
    // one was missed when the other gained it.
    let dir = scratch_dir("limits");
    let a = dir.join("a.json");
    let _ = std::fs::write(
        &a,
        r#"{"lane":"Security","model":"claude:sonnet","status":{"state":"swept"},"findings":[]}"#,
    );
    let merged = merge(&[a]).unwrap_or_else(|e| panic!("merge failed: {e}"));
    let text = merged.to_text();
    assert!(text.contains("could not see"), "{text}");
    assert!(text.contains("No finding here came from running"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_merged_report_of_an_unsandboxable_vendor_carries_the_caution_too() {
    // The limits had exactly this gap: added to one renderer and missed in
    // the other, so the same report read differently depending on which
    // command produced it.
    let dir = scratch_dir("kilo-caution");
    let a = dir.join("a.json");
    let _ = std::fs::write(
        &a,
        r#"{"lane":"Security","model":"kilo:kimi","status":{"state":"swept"},"findings":[]}"#,
    );
    let merged = merge(&[a]).unwrap_or_else(|e| panic!("merge failed: {e}"));
    assert!(
        merged.to_text().contains("Caution:"),
        "{}",
        merged.to_text()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sweep that ran out of turns must say so after a merge, too.
///
/// The defect: `judge`'s own sweep type had no field for the flag, so reading a
/// report file back off disk discarded it — the information was sitting in the
/// JSON and the merged report presented a lane's partial list as its whole one.
#[test]
fn a_recovered_sweep_is_still_called_recovered_after_merging() {
    let dir = scratch("salvaged-merge");
    let path = write(
        &dir,
        "correctness-sonnet.json",
        r#"{
            "lane": "Correctness",
            "model": "claude:sonnet",
            "status": {"state": "swept", "turns": 30, "salvaged": true},
            "findings": [],
            "rejected": []
        }"#,
    );
    let merged = merge(&[path]).expect("merge");
    assert!(
        merged.sources.iter().any(|s| s.salvaged),
        "the flag did not survive being read back off disk"
    );
    let text = merged.to_text();
    assert!(
        text.contains("RECOVERED"),
        "the merged report does not say so:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// And a sweep that finished must not be labelled as cut short.
#[test]
fn a_complete_sweep_is_not_called_recovered() {
    let dir = scratch("salvaged-clean");
    let path = write(
        &dir,
        "correctness-sonnet.json",
        r#"{
            "lane": "Correctness",
            "model": "claude:sonnet",
            "status": {"state": "swept", "turns": 12, "salvaged": false},
            "findings": [],
            "rejected": []
        }"#,
    );
    let text = merge(&[path]).expect("merge").to_text();
    assert!(
        !text.contains("RECOVERED"),
        "a clean sweep was labelled recovered:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
