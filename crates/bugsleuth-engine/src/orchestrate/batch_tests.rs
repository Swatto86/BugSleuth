//! Cancellation must keep a sweep that finished in the race window.

use super::reap_cancelled;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::task::JoinSet;

/// A result that becomes ready only after the non-blocking look would have
/// returned nothing must still reach the parent.
///
/// The defect: cancel drained with `try_join_next`, then dropped the JoinSet.
/// A sweep could finish in that gap — provider work done, result sitting in the
/// set — and be discarded, so a later resume paid for it again.
#[tokio::test]
async fn a_sweep_that_finishes_during_cancellation_is_still_collected() {
    let mut tasks: JoinSet<i32> = JoinSet::new();
    let finished = Arc::new(AtomicBool::new(false));
    let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
    let flag = finished.clone();

    tasks.spawn(async move {
        go_rx.await.ok();
        flag.store(true, Ordering::SeqCst);
        7
    });
    tasks.spawn(async {
        std::future::pending::<()>().await;
        0
    });

    // Cancel arm's first non-blocking look: nothing ready yet.
    assert!(
        tasks.try_join_next().is_none(),
        "precondition: the completing sweep must not already be joinable"
    );

    // Sweep finishes in the window before the JoinSet would have been dropped.
    go_tx.send(()).unwrap();
    while !finished.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;

    let mut out = Vec::new();
    reap_cancelled(&mut tasks, &mut out).await;
    assert_eq!(
        out,
        [7],
        "a completed sweep in the cancellation race window was discarded"
    );
}

/// The cancel arm must await JoinSet results after abort, not only drain.
#[test]
fn cancellation_awaits_joinset_results_after_abort() {
    let source = include_str!("batch.rs");
    let code = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(before, _)| before);
    let cancel_arm = code
        .split("options.cancel.cancelled()")
        .nth(1)
        .and_then(|rest| rest.split("joined = tasks.join_next()").next())
        .expect("cancellation arm");
    assert!(
        cancel_arm.contains("reap_cancelled"),
        "cancellation must reap through the helper that awaits after abort: {cancel_arm}"
    );
    assert!(
        !cancel_arm.contains("try_join_next"),
        "cancellation still uses the non-blocking drain that drops late completions: {cancel_arm}"
    );
    assert!(
        code.contains("tasks.abort_all()"),
        "reap_cancelled must abort in-flight work before awaiting"
    );
    assert!(
        code.contains("tasks.join_next().await"),
        "reap_cancelled must await JoinSet results so completed sweeps are kept"
    );
}

/// A panic must carry the unit's lane and model out of the JoinSet, not a bare
/// error string that note_panicked would have to invent a Correctness gap for.
#[tokio::test]
async fn a_panicking_inner_sweep_reports_its_own_lane_and_model() {
    use bugsleuth_domain::Lane;
    use tokio::task::JoinSet;

    let mut tasks: JoinSet<(Lane, String, String)> = JoinSet::new();
    let lane = Lane::Security;
    let model = "claude:sonnet".to_string();
    tasks.spawn(async move {
        let lane_for_panic = lane;
        let model_for_panic = model.clone();
        let inner = tokio::spawn(async move {
            panic!("sweep exploded");
        });
        let _abort_inner = super::AbortOnDrop(inner.abort_handle());
        match inner.await {
            Ok(()) => unreachable!("inner was meant to panic"),
            Err(error) => (lane_for_panic, model_for_panic, error.to_string()),
        }
    });

    let joined = tasks.join_next().await.expect("task").expect("outer");
    assert_eq!(joined.0, Lane::Security);
    assert_eq!(joined.1, "claude:sonnet");
    assert!(
        joined.2.contains("panicked") || joined.2.contains("sweep exploded"),
        "expected a panic error, got {}",
        joined.2
    );
}
