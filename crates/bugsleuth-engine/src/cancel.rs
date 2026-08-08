//! Stopping a run that is already under way.
//!
//! A run is tens of minutes and costs real subscription quota, and until now the
//! only way out of one started against the wrong repository — or with a far
//! bigger matrix than intended — was to kill the application. That is the exact
//! shape of defect the UX lane exists to find, and it found it here.
//!
//! Cancelling has to do two things that pull against each other. It must stop
//! spending immediately, including killing CLI processes already running, or it
//! is not a cancel at all. And it must keep everything already paid for: sweeps
//! that finished are on disk and a later `--resume` should still find them.
//!
//! The kill comes free. Every CLI is spawned with `kill_on_drop`, so dropping
//! the future that is awaiting one takes the child with it — which is why this
//! is a signal to stop awaiting rather than a message passed down to each
//! adapter.

use tokio::sync::watch;

/// A shared "stop now" signal, cloneable and cheap.
///
/// Backed by a `watch` channel rather than an `AtomicBool` plus a `Notify`. The
/// pair had a lost-wakeup window: `cancelled` read the flag and only *then*
/// registered for the notification, so a `stop` landing in that gap set the flag
/// against no registered waiter and the wait then blocked forever on a second
/// notification that never came. A `watch` receiver combines the retained latest
/// value with the wakeup atomically, so there is no gap to lose.
#[derive(Clone)]
pub struct Cancel {
    stopped: watch::Sender<bool>,
}

impl Default for Cancel {
    fn default() -> Self {
        let (stopped, _) = watch::channel(false);
        Self { stopped }
    }
}

impl Cancel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the run to stop. Safe to call repeatedly and from any thread.
    pub fn stop(&self) {
        self.stopped.send_replace(true);
    }

    #[must_use]
    pub fn stopped(&self) -> bool {
        *self.stopped.borrow()
    }

    /// Resolves once cancellation is asked for, immediately if it already was.
    pub async fn cancelled(&self) {
        let mut stopped = self.stopped.subscribe();
        loop {
            // Subscribe, then check: a stop that already happened is visible
            // here, and a stop that happens between this check and `changed`
            // bumps the version so `changed` returns at once. No wakeup is lost.
            if *stopped.borrow_and_update() {
                return;
            }
            if stopped.changed().await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_waiter_started_before_the_stop_is_woken_by_it() {
        let cancel = Cancel::new();
        let waiting = cancel.clone();
        let task = tokio::spawn(async move { waiting.cancelled().await });
        // Give the task a chance to actually park on the notification.
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.stop();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .is_ok(),
            "a task already waiting was never woken"
        );
    }

    #[tokio::test]
    async fn a_waiter_started_after_the_stop_does_not_wait_at_all() {
        // The race this guards: notify_waiters only wakes tasks already parked,
        // so a task that arrives late would wait forever for a notification
        // that has already been sent. The flag is what saves it.
        let cancel = Cancel::new();
        cancel.stop();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), cancel.cancelled())
                .await
                .is_ok(),
            "a task that arrived after the stop waited for a second one"
        );
    }

    #[tokio::test]
    async fn nothing_is_cancelled_until_it_is_asked_for() {
        let cancel = Cancel::new();
        assert!(!cancel.stopped());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), cancel.cancelled())
                .await
                .is_err(),
            "a run cancelled itself with nobody asking"
        );
    }

    #[test]
    fn stopping_twice_is_harmless() {
        let cancel = Cancel::new();
        cancel.stop();
        cancel.stop();
        assert!(cancel.stopped());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_stop_racing_the_wait_is_never_lost() {
        // The lost-wakeup this type was rewritten to remove: a stop that lands
        // in the instant between checking the flag and parking on the wake would
        // hang the waiter forever. Racing the two many times on several threads
        // hits that window; every waiter must still finish.
        for _ in 0..3000 {
            let cancel = Cancel::new();
            let waiter = cancel.clone();
            let task = tokio::spawn(async move { waiter.cancelled().await });
            cancel.stop();
            assert!(
                tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .is_ok(),
                "a stop racing the wait was lost and the waiter hung"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn one_stop_wakes_every_waiter() {
        let cancel = Cancel::new();
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let waiter = cancel.clone();
            tasks.push(tokio::spawn(async move { waiter.cancelled().await }));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.stop();
        for task in tasks {
            assert!(
                tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .is_ok(),
                "a simultaneous waiter was not woken by the single stop"
            );
        }
    }
}
