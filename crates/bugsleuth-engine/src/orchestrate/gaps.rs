//! Everything a run says about what did *not* happen.
//!
//! Three moments, one subject. Before any quota is spent, the cautions that
//! inform a choice still free to make. After it, the sweeps a cancellation
//! stopped and the sweeps whose task died. All three exist because this tool's
//! central discipline is that an absent finding is never silent — a lane that
//! failed must never read like a lane that found nothing.

use bugsleuth_domain::Lane;

use super::{Gap, RunOptions};
use crate::plan::{Plan, Unit};

/// Everything worth saying before any quota is spent.
///
/// Both of these inform a choice the reader can still make for free — commit
/// first, or point this vendor somewhere else — and neither is worth anything
/// once twelve sweeps have run. Grouped because they are one moment in the run,
/// not because they are one subject.
pub(super) fn caution(plan: &Plan, repo: &std::path::Path) {
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
pub(super) fn note_cancelled(options: &RunOptions<'_>, remaining: &[Unit], gaps: &mut Vec<Gap>) {
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

/// Name every sweep whose task died outright.
///
/// The comment beside the `JoinSet` demanded this for weeks while the code only
/// printed a warning, so a panicking sweep simply vanished from the report —
/// which reads exactly like a lane that ran and found nothing. Found by this
/// tool reviewing itself.
///
/// The lane is unknown by the time a task has panicked, so these are recorded
/// against Correctness with the error, rather than dropped for want of a
/// perfect label.
pub(super) fn note_panicked(panicked: &[String], gaps: &mut Vec<Gap>) {
    for error in panicked {
        gaps.push(Gap {
            lane: Lane::Correctness,
            model: None,
            reason: format!("a sweep failed to complete and produced nothing: {error}"),
        });
    }
}
