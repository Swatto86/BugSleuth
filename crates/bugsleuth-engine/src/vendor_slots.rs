//! How many of each vendor's CLI processes may run at once.
//!
//! Most of these CLIs share mutable authentication and session state on disk,
//! so two of one vendor's processes are not two independent programs — which is
//! why every vendor but one is still one-at-a-time across the whole
//! application, repository batches included.
//!
//! Claude is the exception, and it is a measured one rather than an optimistic
//! one. Each invocation is given its own session id before launch, its prompt
//! arrives on stdin, its answer comes back on stdout, and `--safe-mode` stops
//! it loading anything from the machine or the repository. Nothing about one
//! invocation is written where another can read it, so the only real bound is
//! the account's own rate limit — which is a number the user knows and this
//! tool does not, so it is a setting rather than a constant.

use crate::sweep::Vendor;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{AcquireError, Mutex, MutexGuard, Semaphore, SemaphorePermit};

/// The most Claude sessions this application will run at once, whatever is
/// asked for. A ceiling rather than a recommendation: past this the account's
/// rate limit is reached long before the machine's, and every rejected sweep
/// still costs the wait before it fails.
pub const MAX_CLAUDE_SESSIONS: usize = 8;

/// Sessions used when nothing has been configured — the command line, and a
/// desktop settings file written before this existed.
pub const DEFAULT_CLAUDE_SESSIONS: usize = 3;

/// A resizable pool of interchangeable sessions.
///
/// The count is kept alongside the semaphore rather than derived from it,
/// because `available_permits` answers how many are free right now, not how
/// many exist — a pool of three with two sweeps running would read as one, and
/// the next resize would compute its difference from that.
struct Sessions {
    free: Semaphore,
    /// How many the pool has been sized to. Read by [`capacity`], which is how
    /// the planner knows how many Claude sweeps go in one concurrent group.
    limit: AtomicUsize,
    /// One resize at a time, so two callers cannot both read the old count and
    /// each apply the difference from it.
    resize: std::sync::Mutex<()>,
}

impl Sessions {
    const fn new(limit: usize) -> Self {
        Self {
            free: Semaphore::const_new(limit),
            limit: AtomicUsize::new(limit),
            resize: std::sync::Mutex::new(()),
        }
    }

    /// Resize the pool, returning the limit actually in force afterwards.
    ///
    /// The returned number is the honest one. A permit that is currently held
    /// cannot be taken back — the sweep holding it is already talking to the
    /// CLI — so shrinking while work is running shrinks as far as it can and
    /// reports that, rather than claiming a bound it is not enforcing. Callers
    /// set this while idle, where the request always applies exactly.
    fn resize(&self, requested: usize) -> usize {
        let target = requested.clamp(1, MAX_CLAUDE_SESSIONS);
        // A poisoned lock means an earlier caller panicked mid-resize. The
        // permit count is still consistent — each branch below is one call — so
        // recovering beats refusing every resize for the rest of the process.
        let _exclusive = self
            .resize
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = self.limit.load(Ordering::SeqCst);
        let applied = match target.cmp(&current) {
            std::cmp::Ordering::Greater => {
                self.free.add_permits(target - current);
                target
            }
            std::cmp::Ordering::Less => current - self.free.forget_permits(current - target),
            std::cmp::Ordering::Equal => current,
        };
        self.limit.store(applied, Ordering::SeqCst);
        applied
    }

    fn limit(&self) -> usize {
        self.limit.load(Ordering::SeqCst)
    }
}

static CLAUDE: Sessions = Sessions::new(DEFAULT_CLAUDE_SESSIONS);

/// Set how many Claude CLI sessions may run at once, returning what was applied.
///
/// See [`Sessions::resize`] for why the answer can differ from the request.
pub fn set_claude_sessions(requested: usize) -> usize {
    CLAUDE.resize(requested)
}

/// How many Claude sessions may run at once, as last applied.
#[must_use]
pub fn claude_sessions() -> usize {
    CLAUDE.limit()
}

/// How many sweeps of one vendor may be in flight together.
///
/// The planner groups units by this. It has to agree with what [`acquire`]
/// actually hands out: a group of four Claude sweeps against a gate of one is
/// not four concurrent sweeps, it is three of them waiting with the progress
/// display claiming otherwise.
pub(crate) fn capacity(vendor: Vendor) -> usize {
    match vendor {
        Vendor::Claude => claude_sessions(),
        _ => 1,
    }
}

/// A reserved slot. Held for the length of one CLI invocation and released on
/// drop, including when the task holding it is cancelled.
///
/// Neither guard is ever read — holding one *is* the reservation, and dropping
/// it *is* the release — so the compiler's "never read" is accurate and the
/// value is still doing its whole job.
#[expect(
    dead_code,
    reason = "each variant's guard reserves the slot by existing"
)]
pub(crate) enum Slot {
    /// A vendor that runs one process at a time.
    Serial(MutexGuard<'static, ()>),
    /// One of several sessions this vendor may run together.
    Session(SemaphorePermit<'static>),
}

static CODEX: Mutex<()> = Mutex::const_new(());
static CURSOR: Mutex<()> = Mutex::const_new(());
static OPENCODE: Mutex<()> = Mutex::const_new(());

/// Codex applies use ephemeral sessions and private answer files. Bound their
/// concurrency without serializing independent repositories behind a lock.
static CODEX_APPLIES: Semaphore = Semaphore::const_new(3);

/// Reserve a slot for one sweep or triage pass.
///
/// # Errors
/// Only if a semaphore has been closed. Nothing here closes one, so this is
/// reported rather than asserted: the workspace forbids a panic on a production
/// path, and a wrong "never happens" is exactly the assumption that becomes a
/// crash in front of a user.
pub(crate) async fn acquire(vendor: Vendor) -> Result<Slot, AcquireError> {
    Ok(match vendor {
        Vendor::Claude => Slot::Session(CLAUDE.free.acquire().await?),
        Vendor::Codex => Slot::Serial(CODEX.lock().await),
        Vendor::Cursor => Slot::Serial(CURSOR.lock().await),
        Vendor::OpenCode => Slot::Serial(OPENCODE.lock().await),
    })
}

/// Reserve a slot for one apply.
///
/// Applies are bounded separately because what makes them safe is different:
/// Codex's are ephemeral sessions writing private answer files, and Claude's
/// are the same isolated invocations a sweep uses with edit permissions added.
/// Each still edits one repository, and two applies may never share one — that
/// is the desktop shell's reservation, not this bound.
///
/// # Errors
/// As [`acquire`].
pub(crate) async fn acquire_apply(vendor: Vendor) -> Result<Slot, AcquireError> {
    Ok(match vendor {
        Vendor::Codex => Slot::Session(CODEX_APPLIES.acquire().await?),
        other => return acquire(other).await,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resizing cannot lie about the bound it applied.
    ///
    /// Run against its own pool rather than the shared `CLAUDE` one, because
    /// the rest of this crate's tests share a process and run real sweeps
    /// through it; a test that assumed the global pool was idle failed
    /// whenever it was not.
    #[tokio::test]
    async fn sessions_resize_and_report_what_was_actually_applied() {
        static POOL: Sessions = Sessions::new(DEFAULT_CLAUDE_SESSIONS);
        assert_eq!(POOL.limit(), DEFAULT_CLAUDE_SESSIONS);
        assert_eq!(POOL.resize(2), 2);
        let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(POOL.free.acquire(), POOL.free.acquire())
        })
        .await
        .expect("two sessions must be held together at a limit of two");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), POOL.free.acquire())
                .await
                .is_err(),
            "a third session ran while the limit was two"
        );
        // Both permits are out, so nothing can be forgotten and the applied
        // limit must be reported as unchanged rather than as the request.
        assert_eq!(POOL.resize(1), 2);
        drop(first);
        // One is free now, so the outstanding reduction takes effect.
        assert_eq!(POOL.resize(1), 1);
        drop(second);
        assert_eq!(
            POOL.resize(DEFAULT_CLAUDE_SESSIONS),
            DEFAULT_CLAUDE_SESSIONS
        );
        // Out-of-range requests are clamped rather than refused, so neither a
        // hand-edited settings file nor a zero can stall every Claude sweep.
        assert_eq!(POOL.resize(0), 1);
        assert_eq!(POOL.resize(1_000), MAX_CLAUDE_SESSIONS);
    }

    /// Only Claude may overlap; the planner is told the same numbers.
    #[test]
    fn capacity_matches_the_gate_each_vendor_is_actually_given() {
        assert_eq!(capacity(Vendor::Claude), claude_sessions());
        assert_eq!(capacity(Vendor::Codex), 1);
        assert_eq!(capacity(Vendor::Cursor), 1);
        assert_eq!(capacity(Vendor::OpenCode), 1);
    }

    #[tokio::test]
    async fn codex_applies_overlap_with_a_bound_and_release_cancelled_waiters() {
        let (first, second, third) =
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                tokio::join!(
                    acquire_apply(Vendor::Codex),
                    acquire_apply(Vendor::Codex),
                    acquire_apply(Vendor::Codex)
                )
            })
            .await
            .expect("three Codex applies must acquire slots together");
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                acquire_apply(Vendor::Codex)
            )
            .await
            .is_err()
        );
        drop(first);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                acquire_apply(Vendor::Codex)
            )
            .await
            .is_ok()
        );
        drop((second, third));
    }

    #[tokio::test]
    async fn different_vendors_overlap_but_same_vendor_waits_and_cancellation_releases() {
        let first = acquire(Vendor::Cursor).await;
        let other = acquire(Vendor::OpenCode).await;
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                acquire(Vendor::Cursor)
            )
            .await
            .is_err()
        );
        drop(first);
        let next =
            tokio::time::timeout(std::time::Duration::from_secs(1), acquire(Vendor::Cursor)).await;
        assert!(next.is_ok());
        drop(other);
    }
}
