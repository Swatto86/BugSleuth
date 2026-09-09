//! Running one round of sweeps, and stopping it.
//!
//! Split from `orchestrate` at the hard line cap, along the seam that was
//! already there: this file is about getting concurrent sweeps run and
//! cancelled, and everything left is about assembling a report from what
//! they returned.

use anyhow::Result;
use bugsleuth_domain::{Finding, Lane};
use std::collections::HashMap;
use tokio::task::{Id, JoinError, JoinSet};

use super::persist::{file_name_for, write_report};
use super::{Gap, RunEvent, RunOptions, Swept, emit, fail_unless_persisted, strike_off};
use crate::plan::{Plan, Unit};
use crate::report::Status;
use crate::sweep;

pub(super) struct SweepOutcome {
    lane: Lane,
    lane_report: crate::report::LaneReport,
    file_name: Option<String>,
}

/// Abort what is still running, then await every JoinSet result.
///
/// A non-blocking `try_join_next` drain left a window: a sweep could finish
/// after the drain saw nothing and before the set was dropped, and its result
/// was discarded even though the provider work had completed. Aborting first
/// does not wait out provider timeouts, and awaiting still collects anything
/// that completed during the cancellation race.
pub(super) async fn reap_cancelled<T: 'static>(
    tasks: &mut JoinSet<T>,
) -> Vec<Result<(Id, T), JoinError>> {
    tasks.abort_all();
    let mut joined = Vec::new();
    while let Some(result) = tasks.join_next_with_id().await {
        joined.push(result);
    }
    joined
}

fn take_joined_result(
    result: Result<(Id, SweepOutcome), JoinError>,
    identities: &mut HashMap<Id, (Lane, String)>,
    out: &mut Vec<SweepOutcome>,
    panicked: &mut Vec<(Lane, String, String)>,
) {
    match result {
        Ok((id, outcome)) => {
            identities.remove(&id);
            out.push(outcome);
        }
        Err(error) => {
            let identity = identities.remove(&error.id());
            if error.is_cancelled() {
                return;
            }
            eprintln!("warning: a sweep task failed to complete: {error}");
            if let Some((lane, model)) = identity {
                panicked.push((lane, model, error.to_string()));
            }
        }
    }
}

pub(super) async fn run_batch(
    batch: &[Unit],
    options: &RunOptions<'_>,
    panicked: &mut Vec<(Lane, String, String)>,
) -> Vec<SweepOutcome> {
    let mut tasks = JoinSet::new();
    let mut identities = HashMap::new();

    for unit in batch {
        let unit = unit.clone();
        let repo = options.repo.to_path_buf();
        let scope = options.scope.map(str::to_string);
        let api_key = options.api_key.map(str::to_string);
        let (max_turns, timeout) = (options.max_turns, options.timeout);
        let identity = (unit.lane, unit.model.clone());

        let handle = tasks.spawn(async move {
            let lane_report = sweep::run_with_agents(
                sweep::Request {
                    repo: &repo,
                    lane: unit.lane,
                    model: &unit.model,
                    scope: scope.as_deref(),
                    effort: &unit.effort,
                    max_turns,
                    timeout,
                    api_key: api_key.as_deref(),
                    binary: None,
                },
                unit.use_agents,
            )
            .await;

            SweepOutcome {
                lane: unit.lane,
                file_name: Some(file_name_for(&unit)),
                lane_report,
            }
        });
        identities.insert(handle.id(), identity);
    }

    let mut out = Vec::with_capacity(batch.len());
    loop {
        tokio::select! {
            // Cancellation wins the race deliberately: aborting in-flight work
            // is what actually stops the spending. Waiting politely for every
            // sweep would mean waiting the full per-sweep timeout — up to
            // forty-five minutes — after the user asked to stop.
            () = options.cancel.cancelled() => {
                for result in reap_cancelled(&mut tasks).await {
                    take_joined_result(result, &mut identities, &mut out, panicked);
                }
                eprintln!(
                    "cancelled: stopping sweep(s) in flight. Sweeps already finished \
                     are on disk and a later --resume will reuse them."
                );
                break;
            }
            joined = tasks.join_next_with_id() => {
                match joined {
                    None => break,
                    Some(result) => take_joined_result(
                        result,
                        &mut identities,
                        &mut out,
                        panicked,
                    ),
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;

/// Everything working through the batches produced, and what stopped it.
///
/// The accumulation is a struct rather than a handful of `&mut` parameters
/// because these six values are one answer: they are written together, in one
/// loop, and read together when the report is assembled.
pub(super) struct Executed {
    pub(super) findings: Vec<Finding>,
    pub(super) swept: Vec<Swept>,
    pub(super) gaps: Vec<Gap>,
    /// Sweeps whose task died outright. Carried out of the loop so they can be
    /// reported as gaps rather than only logged; lane and model travel with the
    /// error so a panic is not mis-labelled as Correctness.
    pub(super) panicked: Vec<(Lane, String, String)>,
    /// Units that never landed — because the run was stopped, or because the
    /// provider stopped serving it.
    pub(super) remaining: Vec<Unit>,
    /// Why the run gave up, when nobody stopped it.
    pub(super) interrupted: Option<String>,
}

/// Run every batch, in order, until they are done or something ends the run.
///
/// Split out of [`run`] at the function-length limit, along the seam already
/// there: this is the part that spends quota, and everything around it decides
/// what to spend it on and what to say afterwards.
///
/// # Errors
/// Only when a completed sweep could not be written to disk. That is a loss of
/// paid work the user would have to buy again, so it ends the run rather than
/// letting it charge ahead losing more.
pub(super) async fn sweep_batches(remaining: Plan, options: &RunOptions<'_>) -> Result<Executed> {
    let mut findings: Vec<Finding> = Vec::new();
    let mut swept: Vec<Swept> = Vec::new();
    let mut gaps: Vec<Gap> = Vec::new();
    let mut panicked: Vec<(Lane, String, String)> = Vec::new();
    // Durable-write failures from the current batch. Collected rather than
    // printed and forgotten: out_dir explicitly asks for recoverable per-sweep
    // output, so a report that did not reach disk is a failed run, not a
    // warning on a stream the desktop app never shows.
    let mut persistence_errors: Vec<anyhow::Error> = Vec::new();
    // Kept so a cancelled run can name what it never got to. Sweeps remove
    // themselves as they land.
    let mut remaining_units: Vec<Unit> = remaining.units.clone();
    // Why the run gave up, when it was not the user who stopped it.
    let mut interrupted: Option<String> = None;
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

        // Whether this batch bought anything at all. A batch in which every
        // sweep was refused by its provider for the same reason is the shape of
        // a spent allowance, and the batches after it would be refused too.
        let mut refusals = 0usize;
        let mut landed = 0usize;
        // Everything in a batch is a different vendor, so these run at once.
        for report in run_batch(batch, options, &mut panicked).await {
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

            landed += 1;
            match &report.lane_report.status {
                Status::Swept { .. } => {
                    swept.push(Swept::from_report(report.lane, &report.lane_report));
                    findings.extend(report.lane_report.findings);
                }
                Status::NotSwept { reason } => {
                    if bugsleuth_provider::looks_exhausted(reason) {
                        refusals += 1;
                        interrupted.get_or_insert_with(|| reason.clone());
                    }
                    gaps.push(Gap {
                        lane: report.lane,
                        model: Some(report.lane_report.model.clone()),
                        reason: reason.clone(),
                    });
                }
            }
        }

        // Every completed sweep in this batch has had its write attempted. A
        // report that did not reach disk is not recoverable by resume, so the
        // run fails here rather than charging ahead and losing more work that
        // the user would have to pay for again.
        fail_unless_persisted(&mut persistence_errors)?;

        // One refusal is a hole in the report and the run carries on. A batch
        // in which *every* sweep was refused is the allowance itself being
        // spent, and the batches after it would be refused the same way — each
        // costing its own wait and its own retry to learn nothing new. Stopping
        // here loses none of that: every sweep already done is on disk, the
        // refused ones were never recorded as swept, and resuming picks up
        // exactly the units that are left.
        if allowance_spent(landed, refusals, remaining_units.len()) {
            emit(
                &options.progress,
                RunEvent::Interrupted {
                    reason: interrupted.clone().unwrap_or_default(),
                    remaining: remaining_units.len(),
                },
            );
            break;
        }
        // A batch that got anything through means the allowance is not spent,
        // so an earlier isolated refusal must not be reported as the reason the
        // run ended.
        interrupted = None;
    }
    // Only what actually stopped the run. A refusal in the final batch left
    // nothing outstanding, so the run reached its end and is not resumable.
    let interrupted = interrupted.filter(|_| !remaining_units.is_empty());

    Ok(Executed {
        findings,
        swept,
        gaps,
        panicked,
        remaining: remaining_units,
        interrupted,
    })
}

/// Whether a finished batch says the provider will not serve this run any more.
///
/// One refusal is a hole in the report and the run carries on — a single sweep
/// can be unlucky. Every sweep in the batch refused is the allowance itself
/// being spent, and the batches after it would be refused the same way, each
/// costing its own wait and its own retry to learn nothing new.
///
/// `remaining` is what makes this a *stop* rather than a verdict: a refusal in
/// the last batch left nothing outstanding, so the run reached its end and
/// there is nothing to resume.
pub(super) fn allowance_spent(landed: usize, refusals: usize, remaining: usize) -> bool {
    landed > 0 && refusals == landed && remaining > 0
}
