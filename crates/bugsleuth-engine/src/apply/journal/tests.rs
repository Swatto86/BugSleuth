//! Tests for the record that makes an interrupted fix run resumable.

use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-journal-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    dir
}

fn work(prompts: &[(usize, &str)]) -> Vec<Step> {
    prompts
        .iter()
        .map(|(position, prompt)| Step {
            position: *position,
            prompt: (*prompt).to_string(),
        })
        .collect()
}

/// The work orders as `handoff` leaves them on disk, which is where
/// `unfinished` reads them back from to judge whether a journal still applies.
fn write_prompts(dir: &Path, steps: &[Step]) {
    for step in steps {
        std::fs::write(
            dir.join(format!("fix-prompt-{:02}.md", step.position)),
            &step.prompt,
        )
        .expect("write prompt");
    }
}

#[test]
fn a_finished_defect_survives_the_process_that_fixed_it() {
    let dir = scratch("survives");
    let steps = work(&[(1, "fix one"), (2, "fix two"), (3, "fix three")]);
    write_prompts(&dir, &steps);

    let mut first = Journal::open(
        &dir,
        "claude:sonnet",
        &steps,
        Baseline::Commit("aaa".into()),
    );
    assert_eq!(first.completed(), 0);
    first.record(1, "fixed the first").expect("record");
    first.record(2, "fixed the second").expect("record");
    drop(first);

    // A new process, exactly as a relaunched app would.
    let resumed = Journal::open(
        &dir,
        "claude:sonnet",
        &steps,
        Baseline::Commit("zzz".into()),
    );
    assert!(resumed.is_done(1) && resumed.is_done(2));
    assert!(!resumed.is_done(3), "the unfinished defect must run again");
    assert_eq!(resumed.completed(), 2);
    assert_eq!(unfinished(&dir, "claude:sonnet"), Some(2));

    // The baseline is the one the *first* attempt started from, not the commit
    // the repository is at now. Everything measured from it — what changed,
    // what to strip attribution from, what to push — has to cover the two fixes
    // the earlier attempt already committed.
    assert!(
        matches!(resumed.baseline(), Baseline::Commit(commit) if commit == "aaa"),
        "resume adopted a later baseline and would exclude the earlier fixes"
    );

    // And the earlier attempt's account survives, so the finished report
    // describes all three defects rather than only the one fixed after resume.
    assert!(resumed.text().contains("fixed the first"));
    assert!(resumed.text().contains("fixed the second"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_journal_is_never_applied_to_work_it_was_not_written_for() {
    let dir = scratch("contract");
    let steps = work(&[(1, "fix one"), (2, "fix two")]);
    write_prompts(&dir, &steps);
    let mut journal = Journal::open(&dir, "claude:sonnet", &steps, Baseline::Unborn);
    journal.record(1, "fixed the first").expect("record");
    drop(journal);
    assert_eq!(unfinished(&dir, "claude:sonnet"), Some(1));
    // What the window is told must match what the engine will do: a resume
    // offered for the wrong model would promise work the run then repeats.
    assert_eq!(unfinished(&dir, "codex:gpt"), None);

    // Re-swept report: the same position now names a different defect. Resuming
    // past it would leave that defect unfixed while reporting it done.
    let reswept = work(&[(1, "fix something else"), (2, "fix two")]);
    let fresh = Journal::open(&dir, "claude:sonnet", &reswept, Baseline::Unborn);
    assert_eq!(fresh.completed(), 0, "a changed defect was treated as done");

    // A different fixing model is different work. The defects already fixed by
    // the old model are the ones the user is least likely to want kept.
    let other = Journal::open(&dir, "codex:gpt", &steps, Baseline::Unborn);
    assert_eq!(other.completed(), 0, "another model inherited the progress");

    // The original work orders still resume.
    let same = Journal::open(&dir, "claude:sonnet", &steps, Baseline::Unborn);
    assert_eq!(same.completed(), 1);

    // Once the prompts on disk are the re-swept ones, the same model has no
    // resume to offer either.
    write_prompts(&dir, &reswept);
    assert_eq!(unfinished(&dir, "claude:sonnet"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_corrupt_journal_costs_the_fixes_again_rather_than_refusing_to_run() {
    // The likeliest cause is a process killed mid-write. Refusing to apply
    // anything until someone deletes a file they were never told about is worse
    // than paying for the fixes again.
    let dir = scratch("corrupt");
    let steps = work(&[(1, "fix one")]);
    std::fs::write(dir.join(FILE), "{ this is not json").expect("write corrupt");

    let journal = Journal::open(&dir, "claude:sonnet", &steps, Baseline::Unborn);
    assert_eq!(journal.completed(), 0);
    assert_eq!(unfinished(&dir, "claude:sonnet"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_finished_run_leaves_nothing_to_resume() {
    let dir = scratch("discard");
    let steps = work(&[(1, "fix one")]);
    write_prompts(&dir, &steps);
    let mut journal = Journal::open(&dir, "claude:sonnet", &steps, Baseline::Unborn);
    journal.record(1, "done").expect("record");
    assert_eq!(unfinished(&dir, "claude:sonnet"), Some(1));
    journal.discard();
    assert_eq!(
        unfinished(&dir, "claude:sonnet"),
        None,
        "a completed run still offered a resume"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recording_the_same_defect_twice_does_not_double_count_it() {
    // Resume re-reads the journal and skips finished defects, so this should
    // not happen — but `completed()` is the number the user is told and the
    // number the report uses, and it must never exceed the defects there are.
    let dir = scratch("idempotent");
    let steps = work(&[(1, "fix one")]);
    let mut journal = Journal::open(&dir, "claude:sonnet", &steps, Baseline::Unborn);
    journal.record(1, "done").expect("record");
    journal.record(1, "done again").expect("record");
    assert_eq!(journal.completed(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
