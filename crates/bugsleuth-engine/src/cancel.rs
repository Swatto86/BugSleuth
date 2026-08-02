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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// A shared "stop now" signal, cloneable and cheap.
#[derive(Clone, Default)]
pub struct Cancel {
    stopped: Arc<AtomicBool>,
    woken: Arc<Notify>,
}

impl Cancel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the run to stop. Safe to call repeatedly and from any thread.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        // `notify_waiters` only wakes tasks already waiting, which is why the
        // flag is set first: a task that starts waiting after this call sees
        // the flag and never waits at all.
        self.woken.notify_waiters();
    }

    #[must_use]
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Resolves once cancellation is asked for, immediately if it already was.
    ///
    /// Loops on the notification rather than trusting a single wake: `Notify`
    /// permits spurious wake-ups, and returning "cancelled" from one would
    /// abandon a run nobody asked to stop.
    pub async fn cancelled(&self) {
        while !self.stopped() {
            self.woken.notified().await;
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
}
