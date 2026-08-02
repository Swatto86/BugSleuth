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

use serde::Serialize;

use crate::plan::{Plan, Unit};
use crate::report::Status;
use crate::sweep;

pub(crate) mod persist;
pub mod progress;
pub mod proving;
pub mod render;
use persist::{file_name_for, reusable, write_report};

/// Something that happened during a run, as it happens.
///
/// A sweep takes tens of minutes and a run is several of them, so a front end
/// that only learns the outcome at the end shows a spinner for half an hour.
/// These are emitted as they occur; the command line prints them and the
/// desktop app forwards them to the window.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    /// A round of sweeps is starting. One per vendor at most, so `units` is
    /// what will run concurrently.
    BatchStarted {
        index: usize,
        total: usize,
        units: Vec<String>,
    },
    /// A sweep already paid for by an earlier run was reused rather than repeated.
    Reused { model: String, lane: String },
    /// A sweep finished. `swept` false means it did not run — `reason` says why,
    /// and it must never be presented as "found nothing".
    SweepFinished {
        model: String,
        lane: String,
        findings: usize,
        swept: bool,
        reason: String,
    },
}

/// Where run events go. `None` means nobody is listening.
pub type Progress = Option<tokio::sync::mpsc::UnboundedSender<RunEvent>>;

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
    /// Receives events as the run proceeds. Sends are best-effort: a front end
    /// that has gone away must not stop the run it started.
    pub progress: Progress,
    /// Signal that stops the run. A run costs tens of minutes and real quota,
    /// so one started against the wrong repository must be stoppable without
    /// killing the application.
    pub cancel: crate::cancel::Cancel,
    /// Model that re-grades every severity with the whole report in view.
    /// Empty turns the pass off, leaving each severity as whichever model found
    /// the defect graded it — in isolation, which is measurably unreliable.
    pub triage_model: &'a str,
}

pub struct RunReport {
    pub ranked: Vec<Ranked>,
    /// What the severity triage pass did, including when it did not run.
    pub triage: crate::triage::Outcome,
    pub swept: Vec<Swept>,
    /// Every hole, with why. Both kinds: no model assigned, and sweep failed.
    pub gaps: Vec<Gap>,
}

/// One sweep that ran, and what came of it.
pub struct Swept {
    pub model: String,
    pub lane: Lane,
    pub findings: usize,
    /// True when this sweep's answer was recovered after it ran out of turns.
    /// Recovered work beats a lost lane, but a short list from a reviewer that
    /// was cut off means "as far as it got", not "that is all there is".
    pub salvaged: bool,
    /// Findings the model reported whose quoted code could not be located.
    /// Surfaced because the rate is the headline measure of whether a model's
    /// claims about this repository can be trusted at all.
    pub rejected: usize,
}

pub struct Gap {
    pub lane: Lane,
    pub model: Option<String>,
    pub reason: String,
}

/// Everything worth saying before any quota is spent.
///
/// Both of these inform a choice the reader can still make for free — commit
/// first, or point this vendor somewhere else — and neither is worth anything
/// once twelve sweeps have run. Grouped because they are one moment in the run,
/// not because they are one subject.
fn caution(plan: &Plan, repo: &std::path::Path) {
    // Vendors that must run in a worktree read HEAD; the others read the working
    // tree. On a dirty repository those are different code, so one run would be
    // reviewing two versions at once and the merged report would silently mix
    // them. Say so rather than let the reader assume one consistent review.
    if crate::orchestrate::proving::has_uncommitted_changes(repo)
        && plan
            .units
            .iter()
            .any(|unit| crate::sweep::Vendor::parse(&unit.model).0.needs_isolation())
    {
        eprintln!(
            "warning: the repository has uncommitted changes, and this run includes a\n         \
             vendor that must review a throwaway checkout of HEAD. Those vendors will\n         \
             not see your uncommitted work while the others will, so the merged report\n         \
             would span two versions of the code. Commit or stash first."
        );
    }

    // Said before anything is paid for, because the choice it informs — whether
    // to point this vendor at this code at all — can still be made here.
    if plan
        .units
        .iter()
        .any(|unit| crate::sweep::Vendor::parse(&unit.model).0.needs_isolation())
    {
        eprintln!("warning: {}", bugsleuth_domain::UNSANDBOXED_VENDOR_WARNING);
    }
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

    caution(plan, options.repo);

    let outstanding = take_reusable(plan, &options, &mut findings, &mut swept);

    let remaining = Plan {
        units: outstanding,
        uncovered: vec![],
    };
    // Kept so a cancelled run can name what it never got to. Sweeps remove
    // themselves as they land.
    let mut remaining_units: Vec<Unit> = remaining.units.clone();
    let batches = remaining.batches();
    for (index, batch) in batches.iter().enumerate() {
        emit(
            &options.progress,
            RunEvent::BatchStarted {
                index: index + 1,
                total: batches.len(),
                units: batch
                    .iter()
                    .map(|u| format!("{} x {}", u.model, u.lane.title()))
                    .collect(),
            },
        );

        // Checked between batches as well as during one: a cancel that arrives
        // while a batch is finishing must not start the next.
        if options.cancel.stopped() {
            break;
        }

        // Everything in a batch is a different vendor, so these run at once.
        for report in run_batch(batch, &options).await {
            if let (Some(dir), Some(name)) = (options.out_dir, report.file_name.as_ref())
                && let Err(error) = write_report(dir, name, &report.lane_report)
            {
                eprintln!("warning: {error}");
            }

            emit(
                &options.progress,
                match &report.lane_report.status {
                    Status::Swept { .. } => RunEvent::SweepFinished {
                        model: report.lane_report.model.clone(),
                        lane: report.lane.title().to_string(),
                        findings: report.lane_report.findings.len(),
                        swept: true,
                        reason: String::new(),
                    },
                    Status::NotSwept { reason } => RunEvent::SweepFinished {
                        model: report.lane_report.model.clone(),
                        lane: report.lane.title().to_string(),
                        findings: 0,
                        swept: false,
                        reason: reason.clone(),
                    },
                },
            );

            remaining_units
                .retain(|unit| unit.lane != report.lane || unit.model != report.lane_report.model);

            match &report.lane_report.status {
                Status::Swept { .. } => {
                    swept.push(Swept {
                        model: report.lane_report.model.clone(),
                        lane: report.lane,
                        findings: report.lane_report.findings.len(),
                        rejected: report.lane_report.rejected.len(),
                        salvaged: matches!(
                            report.lane_report.status,
                            Status::Swept { salvaged: true, .. }
                        ),
                    });
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

    // Severity is the only thing that orders the report, so it is graded once
    // more with everything in view before anything is ranked on it.
    let mut clusters = cluster(findings);
    let triage = crate::triage::grade(&mut clusters, &options).await;

    note_cancelled(&options, &remaining_units, &mut gaps);

    Ok(RunReport {
        ranked: rank(clusters),
        triage,
        swept,
        gaps,
    })
}

/// Consume whatever an earlier run already paid for, returning what is left.
///
/// Pulled out of `run` because reuse and execution are separate phases and
/// reading them together obscures both.
fn take_reusable(
    plan: &Plan,
    options: &RunOptions<'_>,
    findings: &mut Vec<Finding>,
    swept: &mut Vec<Swept>,
) -> Vec<Unit> {
    let mut outstanding = Vec::new();
    for unit in &plan.units {
        match reusable(unit, options) {
            Some(previous) => {
                emit(
                    &options.progress,
                    RunEvent::Reused {
                        model: unit.model.clone(),
                        lane: unit.lane.title().to_string(),
                    },
                );
                swept.push(Swept {
                    model: previous.model.clone(),
                    lane: unit.lane,
                    findings: previous.findings.len(),
                    rejected: previous.rejected.len(),
                    salvaged: matches!(previous.status, Status::Swept { salvaged: true, .. }),
                });
                findings.extend(previous.findings);
            }
            None => outstanding.push(unit.clone()),
        }
    }
    outstanding
}

/// Best-effort send. A closed channel means the front end went away, which is
/// not a reason to abandon a run that is already being paid for.
fn emit(progress: &Progress, event: RunEvent) {
    if let Some(sender) = progress {
        let _ = sender.send(event);
    }
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
/// Name every sweep a cancellation prevented.
///
/// A cancelled run must never read as a finished one. Each unit that never ran
/// becomes a gap with its reason, exactly like a lane nobody assigned to — the
/// report's whole discipline is that an absent finding is never silent.
fn note_cancelled(options: &RunOptions<'_>, remaining: &[Unit], gaps: &mut Vec<Gap>) {
    if !options.cancel.stopped() {
        return;
    }
    for unit in remaining {
        gaps.push(Gap {
            lane: unit.lane,
            model: Some(unit.model.clone()),
            reason: "the run was cancelled before this sweep finished".to_string(),
        });
    }
}

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
                effort: &unit.effort,
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
    loop {
        tokio::select! {
            // Cancellation wins the race deliberately: `JoinSet` aborts its
            // tasks when dropped, and every CLI is spawned with `kill_on_drop`,
            // so leaving this loop is what actually stops the spending. Waiting
            // politely for the in-flight sweeps would mean waiting the full
            // per-sweep timeout — up to forty-five minutes — after the user
            // asked to stop.
            () = options.cancel.cancelled() => {
                eprintln!("cancelled: stopping {} sweep(s) in flight. Sweeps already                            finished are on disk and a later --resume will reuse them.",
                          tasks.len());
                break;
            }
            joined = tasks.join_next() => {
                match joined {
                    None => break,
                    Some(Ok(outcome)) => out.push(outcome),
                    // A panicking sweep must not take the run down with it, and
                    // must not vanish either — the caller has to see a gap
                    // where it should be.
                    Some(Err(error)) => {
                        eprintln!("warning: a sweep task failed to complete: {error}");
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "orchestrate/tests.rs"]
mod tests;
