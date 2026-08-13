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
use std::collections::HashMap;
use tokio::task::{Id, JoinError, JoinSet};

pub(super) struct SweepOutcome {
    pub(super) lane: Lane,
    pub(super) lane_report: crate::report::LaneReport,
    pub(super) file_name: Option<String>,
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
