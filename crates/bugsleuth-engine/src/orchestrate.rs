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

mod gaps;
pub(crate) mod persist;
pub mod progress;
pub mod render;
use persist::{reusable, write_report};

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

/// Remove exactly one outstanding unit for a sweep that has just landed.
///
/// One sweep accounts for one unit. The first version removed *every* unit
/// matching the lane and model, so a model configured for three passes had all
/// three struck off when the first finished — and a run cancelled after pass one
/// reported the other two as accounted for, a claim of coverage that never
/// happened, in the summary whose entire job is saying what did not run.
///
/// Which pass is removed does not matter; the units are interchangeable here.
/// How many are removed is the whole point.
fn strike_off(remaining: &mut Vec<Unit>, lane: Lane, model: &str) {
    let mut once = Some(());
    remaining.retain(|unit| {
        let same_sweep = unit.lane == lane && sweep::resolved_label(&unit.model) == model;
        !(same_sweep && once.take().is_some())
    });
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
    /// Whether the run was stopped part-way rather than reaching its end.
    ///
    /// Captured here, in the engine, at the moment the gaps are written — not
    /// sampled by the Tauri layer afterwards. A cancellation arriving after the
    /// engine has already recorded completion must remain completed, and only
    /// the engine knows which of those happened. Stopping mid-run produced an
    /// `Ok(RunReport)` with cancellation gaps and stopping during pre-check
    /// produced an `Err`, so the same Stop action was reported as "Finished" or
    /// "Run failed" purely by timing.
    pub cancelled: bool,
}

/// One sweep that ran, and what came of it.
pub struct Swept {
    pub model: String,
    pub lane: Lane,
    pub commit: Option<String>,
    /// The clean revision this result was pinned to. `None` means unpinned.
    pub cache_revision: Option<String>,
    pub scope: Option<String>,
    pub usage: Option<String>,
    pub findings: usize,
    /// True when this sweep's answer was recovered and may be partial.
    /// Recovered work beats a lost lane, but a short list means "as far as it
    /// got", not "that is all there is".
    pub salvaged: bool,
    /// Findings the model reported whose quoted code could not be located.
    /// Surfaced because the rate is the headline measure of whether a model's
    /// claims about this repository can be trusted at all.
    pub rejected: usize,
}

impl Swept {
    fn from_report(lane: Lane, report: &crate::report::LaneReport) -> Self {
        Self {
            model: report.model.clone(),
            lane,
            commit: report.commit.clone(),
            cache_revision: report.cache_revision.clone(),
            scope: report.scope.clone(),
            usage: report.usage.clone(),
            findings: report.findings.len(),
            rejected: report.rejected.len(),
            salvaged: matches!(&report.status, Status::Swept { salvaged: true, .. }),
        }
    }
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

    gaps::caution(plan, options.repo);

    // Sweeps whose task died outright. Carried out of the batch loop so they
    // can be reported as gaps rather than only logged.
    let mut panicked: Vec<String> = Vec::new();
    // Durable-write failures from the current batch. Collected rather than
    // printed and forgotten: out_dir explicitly asks for recoverable per-sweep
    // output, so a report that did not reach disk is a failed run, not a
    // warning on a stream the desktop app never shows.
    let mut persistence_errors: Vec<anyhow::Error> = Vec::new();

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
        for report in run_batch(batch, &options, &mut panicked).await {
            if let (Some(dir), Some(name)) = (options.out_dir, report.file_name.as_ref())
                && let Err(error) = write_report(dir, name, &report.lane_report)
            {
                persistence_errors.push(error);
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

            // Both sides resolved. A unit configured as `sonnet` produced a
            // report saying `claude:sonnet`, so this comparison was never true
            // and every finished sweep stayed on the outstanding list — a
            // cancelled run reported lanes it had already swept as not reached.
            strike_off(&mut remaining_units, report.lane, &report.lane_report.model);

            match &report.lane_report.status {
                Status::Swept { .. } => {
                    swept.push(Swept::from_report(report.lane, &report.lane_report));
                    findings.extend(report.lane_report.findings);
                }
                Status::NotSwept { reason } => gaps.push(Gap {
                    lane: report.lane,
                    model: Some(report.lane_report.model.clone()),
                    reason: reason.clone(),
                }),
            }
        }

        // Every completed sweep in this batch has had its write attempted. A
        // report that did not reach disk is not recoverable by resume, so the
        // run fails here rather than charging ahead and losing more work that
        // the user would have to pay for again.
        fail_unless_persisted(&mut persistence_errors)?;
    }

    if common_scope(&swept).is_err() {
        anyhow::bail!(
            "completed sweeps reported different review scopes, so they cannot be presented as one run"
        );
    }

    // Severity is the only thing that orders the report, so it is graded once
    // more with everything in view before anything is ranked on it.
    let mut clusters = cluster(findings);
    let triage = crate::triage::grade(&mut clusters, &options).await;

    // Read once, and used for both the gaps and the report's own flag, so the
    // two cannot disagree about whether this run was stopped.
    let cancelled = options.cancel.stopped();
    gaps::note_cancelled(cancelled, &remaining_units, &mut gaps);
    gaps::note_panicked(&panicked, &mut gaps);

    Ok(RunReport {
        ranked: rank(clusters),
        triage,
        swept,
        gaps,
        cancelled,
    })
}

/// Fail the run if any completed sweep in the just-finished batch could not be
/// persisted. `out_dir` is what makes a run recoverable by `--resume`, so a
/// report that never reached disk is a loss of paid work — reported, not
/// swallowed onto a stream the desktop application never shows.
fn fail_unless_persisted(errors: &mut Vec<anyhow::Error>) -> Result<()> {
    if errors.is_empty() {
        return Ok(());
    }
    let details = errors
        .drain(..)
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    anyhow::bail!("one or more completed sweeps could not be saved: {details}")
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
                swept.push(Swept::from_report(unit.lane, &previous));
                findings.extend(previous.findings);
            }
            None => outstanding.push(unit.clone()),
        }
    }
    outstanding
}

fn common_scope(swept: &[Swept]) -> Result<Option<&str>, ()> {
    let Some(first) = swept.first() else {
        return Ok(None);
    };
    if swept.iter().any(|sweep| sweep.scope != first.scope) {
        return Err(());
    }
    Ok(first.scope.as_deref())
}

/// Best-effort send. A closed channel means the front end went away, which is
/// not a reason to abandon a run that is already being paid for.
fn emit(progress: &Progress, event: RunEvent) {
    if let Some(sender) = progress {
        let _ = sender.send(event);
    }
}

mod batch;
use batch::run_batch;

#[cfg(test)]
#[path = "orchestrate/tests.rs"]
mod tests;
