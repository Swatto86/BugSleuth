//! Tests for report persistence and resume, in their own file only because
//! the module plus its tests crossed the hard line cap.

use super::super::persist::*;
use bugsleuth_domain::Lane;
use std::time::Duration;

#[test]
fn a_successful_sweep_is_reused_rather_than_paid_for_twice() {
    let dir = scratch("reuse");
    let report = lane_report(Status::Swept {
        turns: Some(3),
        salvaged: false,
    });
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, true)).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_sweep_is_retried_not_reused() {
    // The usual reason a run died is a rate limit, which is exactly the case
    // worth attempting again. Reusing it would make the failure permanent.
    let dir = scratch("retry-failed");
    let report = lane_report(Status::NotSwept {
        reason: "rate limited".into(),
    });
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, true)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nothing_is_reused_unless_resume_was_asked_for() {
    let dir = scratch("no-resume");
    let report = lane_report(Status::Swept {
        turns: None,
        salvaged: false,
    });
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, false)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_truncated_report_is_swept_again_rather_than_failing_the_run() {
    // A run killed mid-write leaves half a file. The right response is to
    // sweep again, not to refuse to start.
    let dir = scratch("truncated");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(file_name_for(&unit())), r#"{"lane":"Corr"#);
    assert!(reusable(&unit(), &options(&dir, true)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_unit_gets_a_distinct_file_so_sweeps_cannot_overwrite_each_other() {
    let a = file_name_for(&unit());
    let b = file_name_for(&Unit {
        model: "codex:".into(),
        lane: Lane::Correctness,
        effort: String::new(),
        pass: 1,
    });
    let c = file_name_for(&Unit {
        model: "claude:sonnet".into(),
        lane: Lane::Security,
        effort: String::new(),
        pass: 1,
    });
    assert_ne!(a, b);
    assert_ne!(a, c);
}

#[test]
fn every_sweep_writes_to_its_own_file() {
    let a = LaneReport {
        lane: "Correctness".into(),
        model: "claude:sonnet".into(),
        commit: None,
        status: Status::Swept {
            turns: None,
            salvaged: false,
        },
        findings: vec![],
        rejected: vec![],
    };
    let dir = std::env::temp_dir()
        .join("bugsleuth-orchestrate-tests")
        .join(format!("{}", std::process::id()));
    assert!(write_report(&dir, "correctness-claude-sonnet.json", &a).is_ok());
    assert!(dir.join("correctness-claude-sonnet.json").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

fn unit() -> Unit {
    Unit {
        model: "claude:sonnet".into(),
        lane: Lane::Correctness,
        effort: String::new(),
        pass: 1,
    }
}

fn options<'a>(dir: &'a Path, resume: bool) -> RunOptions<'a> {
    RunOptions {
        repo: Path::new("."),
        scope: None,
        max_turns: 10,
        timeout: Duration::from_secs(60),
        api_key: None,
        out_dir: Some(dir),
        resume,
        progress: None,
        triage_model: "",
        cancel: Default::default(),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-resume-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn lane_report(status: Status) -> LaneReport {
    LaneReport {
        lane: "Correctness".into(),
        model: "claude:sonnet".into(),
        commit: None,
        status,
        findings: vec![],
        rejected: vec![],
    }
}

#[test]
fn a_second_pass_writes_beside_the_first_rather_than_over_it() {
    // The whole value of repetition is keeping both results: three
    // identical sweeps of one fixture found five findings each but six
    // between them. Overwriting would buy nothing.
    let first = file_name_for(&unit());
    let second = file_name_for(&Unit { pass: 2, ..unit() });
    assert_ne!(first, second);
    assert!(second.contains("~p2"), "got {second}");
    // A first pass keeps the historical name, so reports written before
    // passes existed still resume.
    assert!(!first.contains("pass"), "got {first}");
}

#[test]
fn an_effort_spelling_the_pass_suffix_cannot_collide_with_a_real_second_pass() {
    // Both of these are reachable from ordinary config values: effort is
    // free text at every entry point. Under plain concatenation they were
    // byte-identical, so whichever sweep finished last silently overwrote
    // the other and resume handed one unit the other's report.
    let second_pass = file_name_for(&Unit { pass: 2, ..unit() });
    let odd_effort = file_name_for(&Unit {
        effort: "pass2".into(),
        ..unit()
    });
    assert_ne!(second_pass, odd_effort);
    // The same trap from the other side: an effort of "p2" against the
    // new-style pass marker.
    let p2_effort = file_name_for(&Unit {
        effort: "p2".into(),
        ..unit()
    });
    assert_ne!(second_pass, p2_effort);
}

#[test]
fn two_model_ids_that_differ_only_in_punctuation_get_different_files() {
    // `codex:a/b` and `codex:a-b` both used to render as `codex-a-b`: one
    // sweep overwrote the other, and a resumed run handed a model the other
    // model's findings while the report claimed the wrong provenance.
    let slash = file_name_for(&Unit {
        model: "codex:a/b".into(),
        ..unit()
    });
    let dash = file_name_for(&Unit {
        model: "codex:a-b".into(),
        ..unit()
    });
    assert_ne!(slash, dash);
}

#[test]
fn an_encoded_name_can_never_reach_outside_the_run_directory() {
    // The encoding exists to be injective, but it must not have bought that
    // by letting a separator or a parent reference through.
    for hostile in ["../../etc/passwd", r"C:\Windows\System32", r"a/b\c", ".."] {
        let name = file_name_for(&Unit {
            model: hostile.to_string(),
            ..unit()
        });
        assert!(!name.contains('/'), "{name}");
        assert!(!name.contains('\\'), "{name}");
        assert!(!name.contains(".."), "{name}");
        assert!(!name.contains(':'), "{name}");
    }
}

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

#[test]
fn a_report_is_replaced_whole_or_not_at_all() {
    // fs::write truncates first, so a process killed mid-write left half a
    // report where a complete one had been. Resume treats an unparseable
    // report as absent, so the cost is paying tens of minutes for the same
    // sweep twice - and losing the first result in the process.
    let dir = scratch("atomic");
    let name = file_name_for(&unit());
    let first = lane_report(Status::Swept {
        turns: Some(1),
        salvaged: false,
    });
    assert!(write_report(&dir, &name, &first).is_ok());

    let second = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    assert!(write_report(&dir, &name, &second).is_ok());

    // The replacement landed, and no staging file was left behind to be
    // read as a report by anything walking the directory.
    let reused = reusable(&unit(), &options(&dir, true));
    assert!(reused.is_some());
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".writing"))
        .collect();
    assert!(leftovers.is_empty(), "staging files left: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
