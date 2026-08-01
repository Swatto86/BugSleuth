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
    let path = options.out_dir?.join(file_name_for(unit));
    let text = std::fs::read_to_string(path).ok()?;
    let report: LaneReport = serde_json::from_str(&text).ok()?;
    // A failed sweep is retried. The usual reason a run died is a rate limit,
    // which is exactly the case worth attempting again.
    matches!(report.status, Status::Swept { .. }).then_some(report)
}

pub(super) fn file_name_for(unit: &Unit) -> String {
    format!(
        "{}-{}.json",
        unit.lane.slug(),
        unit.model
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
    )
}

pub(super) fn write_report(dir: &Path, name: &str, report: &LaneReport) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path: PathBuf = dir.join(name);
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&path, json)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
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
        let report = lane_report(Status::Swept { turns: Some(3) });
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
        let report = lane_report(Status::Swept { turns: None });
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
        });
        let c = file_name_for(&Unit {
            model: "claude:sonnet".into(),
            lane: Lane::Security,
        });
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn every_sweep_writes_to_its_own_file() {
        let a = LaneReport {
            lane: "Correctness".into(),
            model: "claude:sonnet".into(),
            status: Status::Swept { turns: None },
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
            status,
            findings: vec![],
            rejected: vec![],
        }
    }
}
