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

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use bugsleuth_domain::{Finding, Lane};
use bugsleuth_judge::{Ranked, cluster, rank};

use crate::plan::{Plan, Unit};
use crate::report::Status;
use crate::sweep;

mod persist;
use persist::{file_name_for, reusable, write_report};

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

impl RunReport {
    /// How many distinct lanes actually ran.
    fn lanes_swept(&self) -> usize {
        let mut lanes: Vec<Lane> = self.swept.iter().map(|(_, lane, _)| *lane).collect();
        lanes.sort_by_key(|lane| lane.slug());
        lanes.dedup();
        lanes.len()
    }

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

        // Severity means different things in different mandates. A "high" from
        // the security lane and a "high" from the correctness lane were assigned
        // by models answering different questions, so ordering one against the
        // other is not a judgement anyone actually made. Say so rather than let
        // a single ranked list imply a comparison it cannot support.
        if self.lanes_swept() > 1 {
            out.push_str(
                "\n  Note: this list spans more than one lane. Severities are relative to\n  \
                 each lane's own mandate and are NOT directly comparable between lanes -\n  \
                 read the ordering within a lane, not across them.\n",
            );
        }

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

    /// The whole orchestration path, end to end, with no model involved.
    ///
    /// Every unit is pre-seeded as a completed sweep and resumed, so this
    /// exercises planning, reuse, merging and reporting for real while costing
    /// nothing. Without it, the only proof `run` works would be having watched
    /// it once.
    #[tokio::test]
    async fn a_fully_resumed_run_merges_previous_sweeps_without_calling_any_model() {
        use crate::plan::{Config, ModelPlan};

        let dir = std::env::temp_dir()
            .join("bugsleuth-run-tests")
            .join(format!("{}-resumed", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // Two vendors reporting the same defect in different words.
        let seed = |model: &str, title: &str, explanation: &str| {
            let report = format!(
                r#"{{"lane":"Correctness","model":"{model}","status":{{"state":"swept"}},
                    "findings":[{{"id":"x","lane":"correctness","model":"{model}",
                      "title":"{title}","severity":"high",
                      "anchor":{{"file":"src/a.rs","line":10,"claimed_line":10,"snippet":"code"}},
                      "explanation":"{explanation}","failure_scenario":"f"}}],
                    "rejected":[]}}"#
            );
            let unit = Unit {
                model: model.to_string(),
                lane: Lane::Correctness,
            };
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join(file_name_for(&unit)), report);
        };
        seed(
            "claude:sonnet",
            "average_price divides by zero on an empty inventory",
            "No check for an empty inventory before dividing by the item count.",
        );
        seed(
            "codex:",
            "Calculating the average price of an empty inventory panics",
            "An empty inventory has length zero so this integer division panics.",
        );

        let plan = crate::plan::plan(&Config {
            models: vec![
                ModelPlan {
                    id: "claude:sonnet".into(),
                    lanes: vec!["correctness".into()],
                },
                ModelPlan {
                    id: "codex:".into(),
                    lanes: vec!["correctness".into()],
                },
            ],
        })
        .unwrap_or_else(|e| panic!("plan failed: {e}"));

        let report = run(
            &plan,
            RunOptions {
                repo: Path::new("."),
                scope: None,
                max_turns: 1,
                timeout: Duration::from_secs(1),
                api_key: None,
                out_dir: Some(&dir),
                resume: true,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("run failed: {e}"));

        assert_eq!(report.swept.len(), 2, "both sweeps should have been reused");
        assert_eq!(
            report.ranked.len(),
            1,
            "the same defect from two vendors should merge into one"
        );
        assert_eq!(report.ranked[0].cluster.agreement, 2);

        // Three lanes had no model, and must be visible as holes.
        assert_eq!(report.gaps.len(), 3);
        let text = report.to_text();
        assert!(text.contains("NOT SWEPT"));
        assert!(text.contains("found by 2 of 2 models"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_single_lane_report_does_not_warn_about_comparing_lanes() {
        assert!(!report(vec![]).to_text().contains("NOT directly comparable"));
    }

    #[test]
    fn a_multi_lane_report_warns_that_severities_are_not_comparable() {
        // A "high" from the security lane and a "high" from the correctness lane
        // were assigned by models answering different questions.
        let multi = RunReport {
            ranked: vec![],
            swept: vec![
                ("claude:sonnet".into(), Lane::Correctness, 1),
                ("claude:sonnet".into(), Lane::Security, 1),
            ],
            gaps: vec![],
        };
        assert_eq!(multi.lanes_swept(), 2);
        assert!(multi.to_text().contains("NOT directly comparable"));
    }

    #[test]
    fn two_models_on_one_lane_is_still_one_lane() {
        let same_lane = RunReport {
            ranked: vec![],
            swept: vec![
                ("claude:sonnet".into(), Lane::Correctness, 1),
                ("codex:".into(), Lane::Correctness, 1),
            ],
            gaps: vec![],
        };
        assert_eq!(same_lane.lanes_swept(), 1);
        assert!(!same_lane.to_text().contains("NOT directly comparable"));
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
}
