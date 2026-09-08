use super::*;
use bugsleuth_engine::cancel::Cancel;
use std::sync::{Arc, Barrier};
use std::thread;

/// Race two idle-to-active transitions against one shared, idle control and
/// report which won, as `(first_ok, second_ok)`. A `Barrier` lines the two
/// threads up so both attempt the transition at once.
fn race(
    first: impl Fn(&RunControl) -> Result<(), String> + Send + 'static,
    second: impl Fn(&RunControl) -> Result<(), String> + Send + 'static,
) -> (Arc<RunControl>, bool, bool) {
    let control = Arc::new(RunControl::default());
    let barrier = Arc::new(Barrier::new(2));

    let c1 = Arc::clone(&control);
    let b1 = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        b1.wait();
        first(&c1).is_ok()
    });
    let c2 = Arc::clone(&control);
    let b2 = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        b2.wait();
        second(&c2).is_ok()
    });
    let first_ok = t1.join().unwrap();
    let second_ok = t2.join().unwrap();
    (control, first_ok, second_ok)
}

#[test]
fn run_control_run_and_apply_cannot_both_start() {
    for _ in 0..200 {
        let (control, run_ok, apply_ok) = race(
            |c| c.try_start_run(Cancel::new()),
            |c| c.try_start_apply(Path::new("/repo"), Cancel::new()),
        );
        assert!(run_ok ^ apply_ok, "exactly one of run/apply must win");
        if run_ok {
            assert!(control.running() && !control.applying());
        } else {
            assert!(control.applying() && !control.running());
        }
    }
}

#[test]
fn run_control_only_one_of_two_runs_can_start() {
    for _ in 0..200 {
        let (control, first, second) = race(
            |c| c.try_start_run(Cancel::new()),
            |c| c.try_start_run(Cancel::new()),
        );
        assert!(first ^ second, "a second run started over the first");
        assert!(control.running());
    }
}

#[test]
fn run_control_clear_and_run_cannot_both_start() {
    for _ in 0..200 {
        let (control, clear_ok, run_ok) =
            race(|c| c.try_start_clear(), |c| c.try_start_run(Cancel::new()));
        assert!(clear_ok ^ run_ok, "clear and run both started");
        assert!(control.running() || control.clearing());
    }
}

#[test]
fn finishing_clears_only_its_own_state() {
    // A stray completion from one operation must not idle another that
    // started after it.
    let control = RunControl::default();
    control
        .try_start_apply(Path::new("/repo"), Cancel::new())
        .expect("apply should start from idle");
    control.finish_run();
    assert!(control.applying(), "finish_run wrongly idled a live apply");
    control.finish_apply(Path::new("/repo"));
    assert!(!control.running() && !control.applying() && !control.clearing());
}

#[test]
fn cancelling_an_apply_stops_its_signal() {
    let control = RunControl::default();
    let cancel = Cancel::new();
    control
        .try_start_apply(Path::new("/repo"), cancel.clone())
        .expect("apply should start from idle");
    control.cancel_apply();
    assert!(
        cancel.stopped(),
        "cancel_apply did not stop the applying signal"
    );
}

#[test]
fn update_cannot_overlap_repository_work_in_either_direction() {
    let control = RunControl::default();
    control
        .try_start_update()
        .expect("update should start while idle");
    assert!(control.try_start_run(Cancel::new()).is_err());
    assert!(
        control
            .try_start_apply(Path::new("/repo"), Cancel::new())
            .is_err()
    );
    assert!(control.try_start_clear().is_err());
    control.finish_update();

    control
        .try_start_clear()
        .expect("clear should start after update releases the state");
    assert!(control.try_start_update().is_err());
}

#[test]
fn separate_applies_finish_independently_and_stop_cancels_all_remaining() {
    let control = RunControl::default();
    let first = Cancel::new();
    let second = Cancel::new();
    control
        .try_start_apply(Path::new("/first"), first.clone())
        .unwrap();
    control
        .try_start_apply(Path::new("/second"), second.clone())
        .unwrap();
    assert!(
        control
            .try_start_apply(Path::new("/first"), Cancel::new())
            .is_err()
    );
    assert!(
        control
            .try_start_apply(Path::new("/first/nested"), Cancel::new())
            .is_err()
    );
    control.finish_apply(Path::new("/first"));
    assert!(control.try_start_run(Cancel::new()).is_err());
    assert!(control.try_start_clear().is_err());
    assert!(control.try_start_update().is_err());
    control.cancel_apply();
    assert!(!first.stopped());
    assert!(second.stopped());
    assert!(control.applying());
    control.finish_apply(Path::new("/second"));
    assert!(control.try_start_run(Cancel::new()).is_ok());
}

#[test]
fn linked_worktrees_cannot_apply_together() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-linked-apply-{}", std::process::id()));
    let repo = dir.join("repo");
    let linked = dir.join("linked");
    let metadata = repo.join(".git/worktrees/linked");
    std::fs::create_dir_all(&metadata).unwrap();
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", metadata.display()),
    )
    .unwrap();
    std::fs::write(metadata.join("commondir"), "../..\n").unwrap();
    let control = RunControl::default();
    control.try_start_apply(&repo, Cancel::new()).unwrap();
    assert!(control.try_start_apply(&linked, Cancel::new()).is_err());
    control.finish_apply(&repo);
    assert!(control.try_start_apply(&linked, Cancel::new()).is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}
