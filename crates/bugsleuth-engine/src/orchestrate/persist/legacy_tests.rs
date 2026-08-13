//! Resume through the historical report filenames.
//!
//! Split from `tests.rs` at the hard line cap, along its own seam: those tests
//! cover the rules resume applies, and these cover the three grammars a report
//! may already be stored under. Every candidate found by an old name is still
//! validated against the report's own recorded lane, model, scope and revision,
//! because two of the three encodings were lossy — which is what the last two
//! tests here are about.

use super::super::persist::*;
use super::tests::{lane_report, options, scratch, unit};
use bugsleuth_domain::Lane;

#[test]
fn a_report_written_under_the_old_naming_is_still_reused() {
    // Every report on disk was written by the lossy encoding, and each cost
    // tens of minutes. Changing the encoding must not silently throw them
    // away and charge for the sweeps again.
    let dir = scratch("legacy");
    let report = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    assert!(write_report(&dir, &legacy_file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, true)).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_ambiguous_historical_effort_pass_name_is_not_reused() {
    let dir = scratch("legacy-effort-pass-collision");
    let report = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    let first_pass = Unit {
        model: "sonnet".into(),
        lane: Lane::Correctness,
        effort: "pass2".into(),
        use_agents: false,
        pass: 1,
    };
    let second_pass = Unit {
        effort: String::new(),
        pass: 2,
        ..first_pass.clone()
    };
    let colliding_name = original_lossy_file_name_for(&first_pass);
    assert_eq!(colliding_name, original_lossy_file_name_for(&second_pass));
    assert!(write_report(&dir, &colliding_name, &report).is_ok());
    assert!(
        reusable(&second_pass, &options(&dir, true)).is_none(),
        "an ambiguous first-pass effort was reused as a different pass"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn current_reports_still_resume_nondefault_sweeps() {
    let dir = scratch("current-nondefault");
    let report = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    let unit = Unit {
        model: "sonnet".into(),
        lane: Lane::Correctness,
        effort: "high".into(),
        use_agents: true,
        pass: 2,
    };
    assert!(write_report(&dir, &file_name_for(&unit), &report).is_ok());
    assert!(reusable(&unit, &options(&dir, true)).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Unambiguous reports from the historical escaping grammar are still read.
///
/// Literal filenames, not the compatibility builders, because a builder that
/// reconstructs a historical name incorrectly passes a test written against
/// itself. `runs/selfreview-0802/security-codex_3a.json` is one of these on disk
/// right now: the reader asked for `security-codex_3a_.json` and then
/// `security-codex-.json`, found neither, and charged for the sweep again.
#[test]
fn an_unambiguous_historical_filename_is_still_reused() {
    let dir = scratch("historical-default");
    let mut report = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    report.model = crate::sweep::resolved_label("codex:");
    let unit = Unit {
        model: "codex:".into(),
        lane: Lane::Correctness,
        effort: String::new(),
        use_agents: false,
        pass: 1,
    };
    let name = "correctness-codex_3a.json";
    assert!(write_report(&dir, name, &report).is_ok());
    assert!(
        reusable(&unit, &options(&dir, true)).is_some(),
        "a paid unambiguous sweep stored as {name} was not found"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A colliding legacy name must not let one vendor's report stand in for
/// another's.
///
/// `ends_with` compared only the model half, so a Claude report labelled
/// `claude:codex-foo` satisfied a request for `codex:foo` — the Codex sweep was
/// skipped and Claude's findings were reported as its result.
#[test]
fn legacy_filename_collision_cannot_cross_vendor_model() {
    let dir = scratch("cross-vendor");
    let mut report = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    report.model = "claude:codex-foo".into();
    let unit = Unit {
        model: "codex:foo".into(),
        lane: Lane::Correctness,
        effort: String::new(),
        use_agents: false,
        pass: 1,
    };
    assert!(write_report(&dir, &legacy_file_name_for(&unit), &report).is_ok());
    assert!(
        reusable(&unit, &options(&dir, true)).is_none(),
        "a Claude report was reused as a Codex sweep's result"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_legacy_file_belonging_to_a_different_model_is_not_adopted() {
    // The old encoding could not tell `codex:a/b` from `codex:a-b`, so a
    // legacy file is only believed when the report inside it says it is
    // this sweep. Otherwise resume would hand a model another's findings.
    let dir = scratch("legacy-wrong");
    let mut report = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    report.model = "codex:something-else".into();
    assert!(write_report(&dir, &legacy_file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, true)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_current_name_wins_over_a_legacy_file_for_the_same_unit() {
    let dir = scratch("legacy-precedence");
    let mut current = lane_report(Status::Swept {
        turns: Some(1),
        salvaged: false,
    });
    current.commit = Some("current".into());
    let mut legacy = lane_report(Status::Swept {
        turns: Some(9),
        salvaged: false,
    });
    legacy.commit = Some("legacy".into());
    assert!(write_report(&dir, &file_name_for(&unit()), &current).is_ok());
    assert!(write_report(&dir, &legacy_file_name_for(&unit()), &legacy).is_ok());
    let found = reusable(&unit(), &options(&dir, true));
    assert_eq!(found.and_then(|r| r.commit).as_deref(), Some("current"));
    let _ = std::fs::remove_dir_all(&dir);
}
