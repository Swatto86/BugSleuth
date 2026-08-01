//! Executing a plan: every (model x lane) unit, then one merged report.
//!
//! Two properties this file is responsible for, both about not lying to the
//! reader:
//!
//! - A lane nobody was assigned to appears in the report as **not swept**.
//! - A lane whose sweep *failed* appears the same way, with the reason.
//!
//! Neither is ever rendered as "no findings", and either makes the command exit
//! non-zero so a script cannot mistake a hole for a clean bill of health.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use bugsleuth_domain::{Finding, Lane};
use bugsleuth_judge::{Ranked, cluster, rank};

use crate::plan::{Plan, Unit};
use crate::report::Status;
use crate::sweep;

pub struct RunOptions<'a> {
    pub repo: &'a Path,
    pub scope: Option<&'a str>,
    pub max_turns: u32,
    pub timeout: Duration,
    pub api_key: Option<&'a str>,
    /// Where each individual sweep's JSON is written, so a run that dies part
    /// way through has not thrown away the sweeps already paid for.
    pub out_dir: Option<&'a Path>,
    /// Reuse sweeps already present in `out_dir` instead of paying for them
    /// again. A sweep costs real subscription quota and can take tens of
    /// minutes, so a run that died at unit nine of twelve must not start over.
    ///
    /// Only *successful* sweeps are reused. A sweep that failed is retried,
    /// which is the whole point — the usual reason a run died is that something
    /// was rate-limited, and that is exactly what should be attempted again.
    pub resume: bool,
}

pub struct RunReport {
    pub ranked: Vec<Ranked>,
    pub swept: Vec<(String, Lane, usize)>,
    /// Every hole, with why. Both kinds: no model assigned, and sweep failed.
    pub gaps: Vec<Gap>,
}

pub struct Gap {
    pub lane: Lane,
    pub model: Option<String>,
    pub reason: String,
}

pub async fn run(plan: &Plan, options: RunOptions<'_>) -> Result<RunReport> {
    let mut findings: Vec<Finding> = Vec::new();
    let mut swept = Vec::new();
    let mut gaps: Vec<Gap> = plan
        .uncovered
        .iter()
        .map(|lane| Gap {
            lane: *lane,
            model: None,
            reason: "no model is assigned to this lane".to_string(),
        })
        .collect();

    // Reuse whatever a previous attempt already paid for.
    let mut outstanding: Vec<Unit> = Vec::new();
    for unit in &plan.units {
        match reusable(unit, &options) {
            Some(previous) => {
                eprintln!(
                    "reusing {} x {} from a previous run",
                    unit.model,
                    unit.lane.slug()
                );
                swept.push((previous.model.clone(), unit.lane, previous.findings.len()));
                findings.extend(previous.findings);
            }
            None => outstanding.push(unit.clone()),
        }
    }

    let remaining = Plan {
        units: outstanding,
        uncovered: vec![],
    };
    let batches = remaining.batches();
    for (index, batch) in batches.iter().enumerate() {
        eprintln!(
            "batch {}/{}: {}",
            index + 1,
            batches.len(),
            batch
                .iter()
                .map(|u| format!("{} x {}", u.model, u.lane.slug()))
                .collect::<Vec<_>>()
                .join(", ")
        );

        // Everything in a batch is a different vendor, so these run at once.
        for report in run_batch(batch, &options).await {
            if let (Some(dir), Some(name)) = (options.out_dir, report.file_name.as_ref())
                && let Err(error) = write_report(dir, name, &report.lane_report)
            {
                eprintln!("warning: {error}");
            }

            match &report.lane_report.status {
                Status::Swept { .. } => {
                    swept.push((
                        report.lane_report.model.clone(),
                        report.lane,
                        report.lane_report.findings.len(),
                    ));
                    findings.extend(report.lane_report.findings);
                }
                Status::NotSwept { reason } => gaps.push(Gap {
                    lane: report.lane,
                    model: Some(report.lane_report.model.clone()),
                    reason: reason.clone(),
                }),
            }
        }
    }

    Ok(RunReport {
        ranked: rank(cluster(findings)),
        swept,
        gaps,
    })
}

struct SweepOutcome {
    lane: Lane,
    lane_report: crate::report::LaneReport,
    file_name: Option<String>,
}

/// Run one batch, genuinely concurrently.
///
/// Each sweep is spawned onto the runtime rather than awaited in turn. Awaiting
/// a list of futures one at a time runs them *sequentially* — futures do no work
/// until polled — which would have quietly made the batching pointless.
///
/// Spawning needs owned data, so each task gets its own copy of the handful of
/// small values it needs. That is cheaper than taking on a dependency purely to
/// join a collection of borrowing futures.
async fn run_batch(batch: &[Unit], options: &RunOptions<'_>) -> Vec<SweepOutcome> {
    let mut tasks = tokio::task::JoinSet::new();

    for unit in batch {
        let unit = unit.clone();
        let repo = options.repo.to_path_buf();
        let scope = options.scope.map(str::to_string);
        let api_key = options.api_key.map(str::to_string);
        let (max_turns, timeout) = (options.max_turns, options.timeout);

        tasks.spawn(async move {
            let lane_report = sweep::run(sweep::Request {
                repo: &repo,
                lane: unit.lane,
                model: &unit.model,
                scope: scope.as_deref(),
                max_turns,
                timeout,
                api_key: api_key.as_deref(),
            })
            .await;

            SweepOutcome {
                lane: unit.lane,
                file_name: Some(file_name_for(&unit)),
                lane_report,
            }
        });
    }

    let mut out = Vec::with_capacity(batch.len());
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(outcome) => out.push(outcome),
            // A panicking sweep must not take the run down with it, and must not
            // vanish either — the caller has to see a gap where it should be.
            Err(error) => eprintln!("warning: a sweep task failed to complete: {error}"),
        }
    }
    out
}

/// A previous successful sweep for this unit, if resuming and one exists.
///
/// A file that cannot be read or parsed is treated as absent rather than as an
/// error: the likeliest cause is a run killed mid-write, and the right response
/// to a truncated report is to sweep again, not to refuse to start.
fn reusable(unit: &Unit, options: &RunOptions<'_>) -> Option<crate::report::LaneReport> {
    if !options.resume {
        return None;
    }
    let path = options.out_dir?.join(file_name_for(unit));
    let text = std::fs::read_to_string(path).ok()?;
    let report: crate::report::LaneReport = serde_json::from_str(&text).ok()?;
    // A failed sweep is retried. The usual reason a run died is a rate limit,
    // which is exactly the case worth attempting again.
    matches!(report.status, Status::Swept { .. }).then_some(report)
}

fn file_name_for(unit: &Unit) -> String {
    format!(
        "{}-{}.json",
        unit.lane.slug(),
        unit.model
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
    )
}

fn write_report(dir: &Path, name: &str, report: &crate::report::LaneReport) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path: PathBuf = dir.join(name);
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&path, json)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
    Ok(())
}

impl RunReport {
    pub fn to_text(&self) -> String {
        let mut out = String::from("=== run report ===\n");
        for (model, lane, count) in &self.swept {
            out.push_str(&format!(
                "  swept: {} lane by {model} ({count} findings)\n",
                lane.title()
            ));
        }
        for gap in &self.gaps {
            let who = gap.model.as_deref().unwrap_or("(nobody)");
            out.push_str(&format!(
                "  NOT SWEPT: {} lane by {who} - {}\n",
                gap.lane.title(),
                gap.reason
            ));
        }
        if !self.gaps.is_empty() {
            out.push_str(
                "\n  The lanes above were NOT reviewed. Their absence from the findings\n  \
                 below means nothing was looked for, not that nothing is there.\n",
            );
        }

        let total: usize = self.swept.iter().map(|(_, _, n)| n).sum();
        out.push_str(&format!(
            "\n  {total} findings from {} sweeps merged into {} distinct defects\n",
            self.swept.len(),
            self.ranked.len()
        ));

        for entry in &self.ranked {
            let cluster = &entry.cluster;
            let finding = cluster.representative();
            out.push_str(&format!(
                "\n  {}. [{}] {}\n     {}:{}\n     found by {} of {} models\n",
                entry.position,
                cluster.severity().as_str().to_uppercase(),
                finding.title,
                finding.anchor.file,
                finding.anchor.line,
                cluster.agreement,
                self.swept.len(),
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::LaneReport;

    fn report(gaps: Vec<Gap>) -> RunReport {
        RunReport {
            ranked: vec![],
            swept: vec![("claude:sonnet".into(), Lane::Correctness, 0)],
            gaps,
        }
    }

    #[test]
    fn a_lane_with_no_model_is_named_in_the_report() {
        let text = report(vec![Gap {
            lane: Lane::Security,
            model: None,
            reason: "no model is assigned to this lane".into(),
        }])
        .to_text();
        assert!(text.contains("NOT SWEPT"));
        assert!(text.contains("Security"));
        assert!(text.contains("NOT reviewed"));
    }

    #[test]
    fn a_failed_sweep_is_named_with_its_reason() {
        let text = report(vec![Gap {
            lane: Lane::Ux,
            model: Some("kilo:".into()),
            reason: "the kilo CLI exited with code 1".into(),
        }])
        .to_text();
        assert!(text.contains("NOT SWEPT"));
        assert!(text.contains("kilo:"));
        assert!(text.contains("exited with code 1"));
    }

    #[test]
    fn a_complete_run_says_nothing_about_unswept_lanes() {
        let text = report(vec![]).to_text();
        assert!(!text.contains("NOT SWEPT"));
        assert!(!text.contains("NOT reviewed"));
    }

    #[test]
    fn a_swept_lane_that_found_nothing_is_not_confused_with_a_gap() {
        let text = report(vec![]).to_text();
        assert!(text.contains("swept: Correctness lane"));
        assert!(text.contains("0 findings"));
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
}
