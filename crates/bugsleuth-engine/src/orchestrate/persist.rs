//! Keeping each sweep on disk, and picking one back up.
//!
//! A sweep costs real subscription quota and can take tens of minutes. Writing
//! each one out as it lands is what makes a run that dies at unit nine of twelve
//! recoverable instead of a total loss.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::RunOptions;
use crate::plan::Unit;
use crate::report::{LaneReport, Status};

/// A previous successful sweep for this unit, if resuming and one exists.
///
/// A file that cannot be read or parsed is treated as absent rather than as an
/// error: the likeliest cause is a run killed mid-write, and the right response
/// to a truncated report is to sweep again, not to refuse to start.
pub(super) fn reusable(unit: &Unit, options: &RunOptions<'_>) -> Option<LaneReport> {
    if !options.resume {
        return None;
    }
    let dir = options.out_dir?;
    read_swept(&dir.join(file_name_for(unit)))
        // Reports written before the encoding changed are still worth tens of
        // minutes each, so the old name is tried too — but only after the
        // current one, and only if the report says it is the right sweep. The
        // old encoding was lossy, which is exactly why it is not trusted to
        // identify anything on its own.
        .or_else(|| {
            let legacy = read_swept(&dir.join(legacy_file_name_for(unit)))?;
            let same_sweep =
                legacy.lane == unit.lane.title() && legacy.model.ends_with(model_of(&unit.model));
            same_sweep.then_some(legacy)
        })
}

/// A report at `path`, if it parses and records a sweep that actually ran.
///
/// A file that cannot be read or parsed is treated as absent rather than as an
/// error: the likeliest cause is a run killed mid-write, and the right response
/// to a truncated report is to sweep again, not to refuse to start.
fn read_swept(path: &Path) -> Option<LaneReport> {
    let text = std::fs::read_to_string(path).ok()?;
    let report: LaneReport = serde_json::from_str(&text).ok()?;
    // A failed sweep is retried. The usual reason a run died is a rate limit,
    // which is exactly the case worth attempting again.
    matches!(report.status, Status::Swept { .. }).then_some(report)
}

/// The model half of a `vendor:model` spec. A report records the resolved
/// `vendor:model`, while a unit may hold a bare alias.
fn model_of(spec: &str) -> &str {
    spec.split_once(':').map_or(spec, |(_, model)| model)
}

pub(super) fn file_name_for(unit: &Unit) -> String {
    // A default unit — no effort, first pass — keeps the original name, so
    // every report written before efforts or passes existed stays resumable.
    if unit.effort.trim().is_empty() && unit.pass <= 1 {
        return format!("{}-{}.json", unit.lane.slug(), safe(&unit.model));
    }
    // Anything else appends BOTH fields, delimited by `~` — a character
    // `safe()` can never emit, so no effort or model text can spell a pass
    // marker and no pass marker can spell an effort. Plain `-` concatenation
    // could: effort `pass2` and a real second pass produced byte-identical
    // names, whichever sweep finished last silently overwrote the other, and
    // resume then handed one unit the other unit's report. Both fields always
    // present (even empty) so the split is positional, not guessed.
    format!(
        "{}-{}~{}~p{}.json",
        unit.lane.slug(),
        safe(&unit.model),
        safe(unit.effort.trim()),
        unit.pass.max(1)
    )
}

/// The name a report would have had under the old, lossy encoding.
///
/// Only ever read, never written. A sweep costs tens of minutes, so a run
/// resumed after this changed should still find what it already paid for —
/// but the old encoding could not tell `codex:a/b` from `codex:a-b`, which is
/// why anything found here is checked against the report's own recorded lane
/// and model before it is believed.
fn legacy_file_name_for(unit: &Unit) -> String {
    let lossy = |text: &str| -> String {
        text.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    };
    if unit.effort.trim().is_empty() && unit.pass <= 1 {
        return format!("{}-{}.json", unit.lane.slug(), lossy(&unit.model));
    }
    format!(
        "{}-{}~{}~p{}.json",
        unit.lane.slug(),
        lossy(&unit.model),
        lossy(unit.effort.trim()),
        unit.pass.max(1)
    )
}

/// A filename-safe encoding of one component, from which no two distinct
/// inputs produce the same output.
///
/// Mapping every non-alphanumeric character to a dash was safe but *lossy*:
/// `codex:a/b` and `codex:a-b` both became `codex-a-b`, so two configured
/// models shared one report file. One sweep overwrote the other, and a resumed
/// run handed a model the other model's findings while the merged report
/// stated the wrong provenance.
///
/// Escaping keeps the property that matters — nothing here can be a path
/// separator, a drive letter or `..` — while making the encoding injective:
/// every dash in the output came from a literal dash in the input, and
/// everything else is a hex escape that cannot be produced any other way.
fn safe(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if c == '-' {
            // A literal dash escapes too, or `a-b` and `a/b` would still meet.
            out.push_str("_2d");
        } else {
            // Non-ASCII goes through as its code point, so this stays total.
            out.push_str(&format!("_{:x}", c as u32));
        }
    }
    out
}

/// Write a sweep's report, all of it or none of it.
///
/// Written to a neighbouring temporary file and renamed into place, because
/// `fs::write` truncates first: a process killed mid-write left a half-written
/// report where a complete one had been. Resume already treats an unparseable
/// report as absent, so the cost was not corruption but paying tens of minutes
/// for a sweep twice — and the second time, the first result is gone.
///
/// A rename within one directory is atomic on both platforms this ships to. If
/// the rename fails the temporary file is removed rather than left beside the
/// reports, where its name would not match any unit and it would simply sit
/// there confusing the next reader.
pub(super) fn write_report(dir: &Path, name: &str, report: &LaneReport) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path: PathBuf = dir.join(name);
    let json = serde_json::to_string_pretty(report)?;

    let staged = dir.join(format!(".{name}.writing"));
    std::fs::write(&staged, json)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", staged.display()))?;
    if let Err(error) = std::fs::rename(&staged, &path) {
        let _ = std::fs::remove_file(&staged);
        anyhow::bail!("cannot replace {}: {error}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
