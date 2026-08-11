//! Running one round of sweeps, and stopping it.
//!
//! Split from `orchestrate` at the hard line cap, along the seam that was
//! already there: this file is about getting concurrent sweeps run and
//! cancelled, and everything left is about assembling a report from what
//! they returned.

use super::RunOptions;
use super::persist::file_name_for;
use crate::plan::Unit;
use crate::sweep;
use bugsleuth_domain::Lane;
use tokio::task::{AbortHandle, JoinSet};

pub(super) struct SweepOutcome {
    pub(super) lane: Lane,
    pub(super) lane_report: crate::report::LaneReport,
    pub(super) file_name: Option<String>,
}

/// One JoinSet item: a finished sweep, or a panic with the unit still attached.
///
/// Identity has to survive the JoinSet boundary. A bare `JoinError` string
/// forced every panic gap onto Correctness with no model, so a Security sweep
/// that died pointed the coverage report at the wrong lane.
enum BatchResult {
    Completed(SweepOutcome),
    Panicked {
        lane: Lane,
        model: String,
        error: String,
    },
}

/// When the JoinSet aborts this outer task, abort the inner sweep too.
///
/// `tokio::spawn` inside a JoinSet task is otherwise orphaned on cancel: the
/// outer future is cancelled, the provider CLI keeps spending.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Abort what is still running, then await every JoinSet result.
///
/// A non-blocking `try_join_next` drain left a window: a sweep could finish
/// after the drain saw nothing and before the set was dropped, and its result
/// was discarded even though the provider work had completed. Aborting first
/// does not wait out provider timeouts — it only reaps wrappers — and awaiting
/// still collects anything that completed during the cancellation race.
pub(super) async fn reap_cancelled<T: 'static>(tasks: &mut JoinSet<T>, out: &mut Vec<T>) {
    tasks.abort_all();
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(outcome) => out.push(outcome),
            Err(error) => {
                eprintln!("warning: a sweep task failed to complete: {error}");
            }
        }
    }
}

fn take_batch_result(
    result: BatchResult,
    out: &mut Vec<SweepOutcome>,
    panicked: &mut Vec<(Lane, String, String)>,
) {
    match result {
        BatchResult::Completed(outcome) => out.push(outcome),
        BatchResult::Panicked {
            lane,
            model,
            error,
        } => {
            eprintln!("warning: a sweep task failed to complete: {error}");
            panicked.push((lane, model, error));
        }
    }
}

pub(super) async fn run_batch(
    batch: &[Unit],
    options: &RunOptions<'_>,
    panicked: &mut Vec<(Lane, String, String)>,
) -> Vec<SweepOutcome> {
    let mut tasks = JoinSet::new();

    for unit in batch {
        let unit = unit.clone();
        let repo = options.repo.to_path_buf();
        let scope = options.scope.map(str::to_string);
        let api_key = options.api_key.map(str::to_string);
        let (max_turns, timeout) = (options.max_turns, options.timeout);
        let lane = unit.lane;
        let model = unit.model.clone();

        tasks.spawn(async move {
            let lane_for_panic = lane;
            let model_for_panic = model.clone();
            let inner = tokio::spawn(async move {
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
            let _abort_inner = AbortOnDrop(inner.abort_handle());
            match inner.await {
                Ok(outcome) => BatchResult::Completed(outcome),
                Err(error) => BatchResult::Panicked {
                    lane: lane_for_panic,
                    model: model_for_panic,
                    error: error.to_string(),
                },
            }
        });
    }

    let mut out = Vec::with_capacity(batch.len());
    loop {
        tokio::select! {
            // Cancellation wins the race deliberately: aborting in-flight work
            // is what actually stops the spending. Waiting politely for every
            // sweep would mean waiting the full per-sweep timeout — up to
            // forty-five minutes — after the user asked to stop.
            () = options.cancel.cancelled() => {
                let mut harvested = Vec::new();
                reap_cancelled(&mut tasks, &mut harvested).await;
                for result in harvested {
                    take_batch_result(result, &mut out, panicked);
                }
                eprintln!(
                    "cancelled: stopping sweep(s) in flight. Sweeps already finished \
                     are on disk and a later --resume will reuse them."
                );
                break;
            }
            joined = tasks.join_next() => {
                match joined {
                    None => break,
                    Some(Ok(result)) => take_batch_result(result, &mut out, panicked),
                    // Outer task itself failed before returning a BatchResult
                    // (for example JoinSet abort tearing it down mid-flight).
                    // Identity lives on BatchResult::Panicked; a bare JoinError
                    // here has none left to report.
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
#[path = "batch_tests.rs"]
mod tests;
