//! Everything a run says about what did *not* happen.
//!
//! Three moments, one subject. Before any quota is spent, the cautions that
//! inform a choice still free to make. After it, the sweeps a cancellation
//! stopped and the sweeps whose task died. All three exist because this tool's
//! central discipline is that an absent finding is never silent — a lane that
//! failed must never read like a lane that found nothing.

use bugsleuth_domain::Lane;

use super::Gap;
use crate::plan::{Plan, Unit};

/// Whether git cannot confirm that the repository is clean.
///
/// A sweep by an ordinary vendor reads the **working tree**, but a vendor that
/// must run in isolation (Kilo) reviews a worktree checked out at **HEAD**. On a
/// dirty repository those are different code, so one run would review two
/// versions at once and the merged report would silently span them. Surfacing
/// the mismatch is the whole point of the caution below.
pub(super) fn repository_is_not_confirmed_clean(repo: &std::path::Path) -> bool {
    // `--untracked-files=all` overrides `status.showUntrackedFiles`, which is a
    // presentation setting and has no business deciding a correctness question:
    // with it set to `no`, an untracked source file vanished from this answer
    // and the run silently reviewed two different trees.
    bugsleuth_verify::hide_console_window(&mut std::process::Command::new("git"))
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo)
        .output()
        .map(|out| !out.status.success() || !out.stdout.is_empty())
        .unwrap_or(true)
}

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
    if repository_is_not_confirmed_clean(repo)
        && plan
            .units
            .iter()
            .any(|unit| crate::sweep::Vendor::parse(&unit.model).0.needs_isolation())
    {
        eprintln!(
            "warning: the repository is not confirmed clean, and this run includes a\n         \
             vendor that must review a throwaway checkout of HEAD. Any uncommitted work\n         \
             will be invisible to those vendors while the others see it, so the report\n         \
             could span two versions of the code. Run git status successfully, then\n         \
             commit or stash changes first."
        );
    }

    // Said before anything is paid for, because the choice it informs — whether
    // to point this vendor at this code at all — can still be made here.
    if plan_includes_kilo(plan) {
        eprintln!("warning: {}", bugsleuth_domain::UNSANDBOXED_VENDOR_WARNING);
    }

    // Said here for the same reason as the rest: the choice it informs - scope
    // the review, or route this model somewhere with more room - is free now
    // and costs a whole sweep once the run has started. A Kilo sweep died on
    // context against a 3,500-line project whose documentation outweighed its
    // source, and nothing in the failure named the file responsible.
    if let Some(warning) = crate::bulk::caution(&crate::bulk::measure(repo)) {
        eprintln!("warning: {warning}");
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
/// Whether any unit sweeps with Kilo, the one vendor whose confinement is the
/// user's own configuration rather than a flag BugSleuth passes. Kimi also
/// needs a worktree, but its allowlist is the `--agent-file` this tool hands
/// it, so the Kilo-shaped warning must not fire for a Kimi-only run.
pub(super) fn plan_includes_kilo(plan: &Plan) -> bool {
    plan.units
        .iter()
        .any(|unit| crate::sweep::Vendor::parse(&unit.model).0 == crate::sweep::Vendor::Kilo)
}

/// Name every sweep a cancellation prevented.
///
/// A cancelled run must never read as a finished one. Each unit that never ran
/// becomes a gap with its reason, exactly like a lane nobody assigned to — the
/// report's whole discipline is that an absent finding is never silent.
pub(super) fn note_cancelled(cancelled: bool, remaining: &[Unit], gaps: &mut Vec<Gap>) {
    if !cancelled {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dirty_repository_is_detected_and_a_clean_one_is_not() {
        let dir = std::env::temp_dir()
            .join("bugsleuth-dirty-tests")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .is_ok()
        };
        if !git(&["init", "-q"]) {
            return; // no usable git here; the rest of the suite still covers the logic
        }
        let _ = git(&["config", "user.email", "t@example.invalid"]);
        let _ = git(&["config", "user.name", "test"]);
        let _ = std::fs::write(dir.join("a.txt"), "hello\n");
        let _ = git(&["add", "-A"]);
        let _ = git(&["commit", "-qm", "base"]);
        assert!(
            !repository_is_not_confirmed_clean(&dir),
            "a freshly committed tree is clean"
        );

        let _ = std::fs::write(dir.join("a.txt"), "changed\n");
        assert!(
            repository_is_not_confirmed_clean(&dir),
            "an edited working tree is dirty, and a mixed-version review would silently miss it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Presentation config must not decide a correctness question.
    ///
    /// `status.showUntrackedFiles=no` is a perfectly ordinary local setting. It
    /// hid an untracked source file from this check, so a mixed Kilo/Codex run
    /// reviewed two different trees and the caution this function exists to
    /// produce never appeared.
    #[test]
    fn hidden_untracked_files_still_make_the_repository_dirty() {
        let dir = std::env::temp_dir()
            .join("bugsleuth-hidden-untracked")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .is_ok()
        };
        if !git(&["init", "-q"]) {
            return; // no usable git here; the rest of the suite still covers the logic
        }
        let _ = git(&["config", "user.email", "t@example.invalid"]);
        let _ = git(&["config", "user.name", "test"]);
        let _ = git(&["config", "status.showUntrackedFiles", "no"]);
        let _ = std::fs::write(dir.join("a.txt"), "hello\n");
        let _ = git(&["add", "-A"]);
        let _ = git(&["commit", "-qm", "base"]);
        assert!(
            !repository_is_not_confirmed_clean(&dir),
            "a freshly committed tree is clean even with untracked files hidden"
        );

        let _ = std::fs::create_dir_all(dir.join("src"));
        let _ = std::fs::write(dir.join("src").join("new.rs"), "fn main() {}\n");
        assert!(
            repository_is_not_confirmed_clean(&dir),
            "an untracked source file the local config hides is still work Kilo's \
             HEAD checkout cannot see"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_git_status_is_not_mistaken_for_a_clean_tree() {
        let dir = std::env::temp_dir()
            .join("bugsleuth-failed-git-status-tests")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create non-repository directory");

        assert!(
            repository_is_not_confirmed_clean(&dir),
            "a failed git status must warn rather than silently authorize a mixed-version review"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_unsandboxed_warning_is_about_kilo_not_about_every_isolated_vendor() {
        let unit = |model: &str| crate::plan::Unit {
            model: model.to_string(),
            lane: Lane::Correctness,
            pass: 1,
            effort: String::new(),
            use_agents: false,
        };
        let kimi_only = crate::plan::Plan {
            units: vec![unit("kimi:kimi-code/k3")],
            uncovered: vec![],
        };
        assert!(!plan_includes_kilo(&kimi_only), "a Kimi-only run triggers a warning about Kilo");
        let kilo = crate::plan::Plan {
            units: vec![unit("kilo:kilo/k3")],
            uncovered: vec![],
        };
        assert!(plan_includes_kilo(&kilo));
    }
}
