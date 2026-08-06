//! Tests for the text an apply reports.
//!
//! Every one of these guards the same failure: a step that did not happen
//! reading like one that did. That is not a cosmetic concern here — the text is
//! the only account the user gets of a tool that rewrote their code, published
//! it, and started a release.

use super::*;
use bugsleuth_engine::apply::{ApplyReport, PushOutcome, TagOutcome};

#[test]
fn a_model_that_changed_nothing_cannot_report_success_unchallenged() {
    // The dangerous outcome: a confident "fixed all seven" over a tree git
    // says is untouched. The window shows this text, so the contradiction
    // has to be in it.
    let text = describe(&ApplyReport {
        text: "fixed all seven defects".into(),
        changed_files: vec![],
        commits: 0,
        stripped: vec![],
        attributed: vec![],
        push: PushOutcome::NotRequested,
        tag: TagOutcome::NotRequested,
    });
    assert!(text.contains("no files changed"));
    assert!(text.contains("fixed all seven"));
}

#[test]
fn committed_work_is_reported_as_changed_rather_than_as_nothing() {
    let text = describe(&ApplyReport {
        text: "done".into(),
        changed_files: vec!["src/a.rs".into(), "src/b.rs".into()],
        commits: 2,
        stripped: vec![],
        attributed: vec![],
        push: PushOutcome::NotRequested,
        tag: TagOutcome::NotRequested,
    });
    assert!(text.contains("2 files changed, in 2 commits"), "{text}");
    assert!(text.contains("src/b.rs"));
    assert!(
        text.find("2 files changed") < text.find("The model's own account"),
        "the model's claim came before what git observed"
    );
}

#[test]
fn uncommitted_work_says_so_because_the_reader_has_to_go_and_look() {
    let uncommitted = describe(&ApplyReport {
        text: "done".into(),
        changed_files: vec!["src/a.rs".into()],
        commits: 0,
        stripped: vec![],
        attributed: vec![],
        push: PushOutcome::NotRequested,
        tag: TagOutcome::NotRequested,
    });
    assert!(
        uncommitted.contains("1 file changed, none of it committed"),
        "{uncommitted}"
    );
    assert!(uncommitted.contains("git diff"));
}

#[test]
fn a_commit_that_credits_the_tool_is_named_before_it_is_pushed() {
    // The prompt tells the model not to add an authorship trailer and every
    // CLI adds one by default, so this is reported from the commits rather
    // than taken on trust. Seven of them reached a real repository whose
    // published history had none, and were caught by luck.
    let text = describe(&ApplyReport {
        text: "done".into(),
        changed_files: vec!["src/a.rs".into()],
        commits: 1,
        stripped: vec![],
        attributed: vec!["fix(auth): stop demanding re-sign-in".into()],
        push: PushOutcome::NotRequested,
        tag: TagOutcome::NotRequested,
    });
    assert!(text.contains("1 commit credits a tool"), "{text}");
    assert!(text.contains("fix(auth): stop demanding re-sign-in"));
    assert!(text.contains("published it is in the history for good"));

    let two = describe(&ApplyReport {
        text: "done".into(),
        changed_files: vec!["a".into()],
        commits: 2,
        stripped: vec![],
        attributed: vec!["one".into(), "two".into()],
        push: PushOutcome::NotRequested,
        tag: TagOutcome::NotRequested,
    });
    assert!(two.contains("2 commits credit a tool"), "{two}");

    let clean = describe(&ApplyReport {
        text: "done".into(),
        changed_files: vec!["a".into()],
        commits: 1,
        stripped: vec![],
        attributed: vec![],
        push: PushOutcome::NotRequested,
        tag: TagOutcome::NotRequested,
    });
    assert!(!clean.contains("credit"), "{clean}");
}

#[test]
fn a_push_that_did_not_happen_says_so_rather_than_staying_silent() {
    // The failure this exists to stop: "push after applying" is turned on
    // once and then forgotten, so a refusal nobody is told about reads
    // exactly like a success until someone notices the branch is behind.
    let refused = describe_push(&PushOutcome::Refused(
        "1 commit still credits a tool for the work".into(),
    ));
    assert!(refused.contains("Not pushed"), "{refused}");
    assert!(refused.contains("credits a tool"), "{refused}");

    let failed = describe_push(&PushOutcome::Failed("non-fast-forward".into()));
    assert!(failed.contains("rejected"), "{failed}");
    assert!(failed.contains("nothing was published"), "{failed}");
    // And it must not imply the tool will sort it out on the next attempt.
    assert!(failed.contains("safe in"), "{failed}");

    let nothing = describe_push(&PushOutcome::NothingToPush);
    assert!(nothing.contains("committed nothing"), "{nothing}");
}

#[test]
fn a_completed_push_names_where_it_went_and_that_it_is_permanent() {
    let text = describe_push(&PushOutcome::Pushed {
        branch: "master".into(),
        upstream: "origin/master".into(),
    });
    assert!(text.contains("Pushed master to origin/master"), "{text}");
    assert!(text.contains("published"), "{text}");

    // Off is silent: a line on every apply about a setting nobody enabled
    // is noise, and noise is what stops the lines above being read.
    assert_eq!(describe_push(&PushOutcome::NotRequested), "");
}

#[test]
fn the_push_outcome_reaches_the_text_the_window_shows() {
    // describe_push is only worth anything if describe actually calls it.
    let text = describe(&ApplyReport {
        text: "done".into(),
        changed_files: vec!["src/a.rs".into()],
        commits: 1,
        stripped: vec![],
        attributed: vec![],
        push: PushOutcome::Pushed {
            branch: "main".into(),
            upstream: "origin/main".into(),
        },
        tag: TagOutcome::Tagged {
            tag: "v1.4.2".into(),
            remote: "origin".into(),
        },
    });
    assert!(text.contains("Pushed main to origin/main"), "{text}");
    // Before the model's account, like everything else git observed.
    assert!(text.find("Pushed main") < text.find("The model's own account"));

    // describe_tag is worth nothing unless describe calls it too, and the
    // release has to be named after the push that made it possible — read
    // the other way round it says a release was cut before its commits
    // were published.
    assert!(text.contains("Tagged v1.4.2"), "{text}");
    assert!(
        text.find("Pushed main") < text.find("Tagged v1.4.2"),
        "{text}"
    );
    assert!(text.find("Tagged v1.4.2") < text.find("The model's own account"));
}

#[test]
fn a_release_that_was_not_tagged_says_so_rather_than_staying_silent() {
    // Same failure as the push: a setting turned on once and forgotten, and
    // a refusal nobody is told about reads exactly like a release that
    // happened — until someone goes looking for it and there is none.
    let refused = describe_tag(&TagOutcome::Refused(
        "v1.0.1 already exists, so nothing was tagged.".into(),
    ));
    assert!(refused.contains("No release was tagged"), "{refused}");
    assert!(refused.contains("already exists"), "{refused}");

    let unpublished = describe_tag(&TagOutcome::NotPushed);
    assert!(
        unpublished.contains("nothing was published"),
        "{unpublished}"
    );

    let failed = describe_tag(&TagOutcome::Failed("permission denied".into()));
    assert!(failed.contains("no release was started"), "{failed}");
    // And it must not imply the commits went with it.
    assert!(failed.contains("commits are pushed"), "{failed}");

    // Off is silent, for the same reason the push is: a line on every apply
    // about a setting nobody enabled is what stops the others being read.
    assert_eq!(describe_tag(&TagOutcome::NotRequested), "");
}

#[test]
fn a_tagged_release_names_it_and_does_not_claim_the_build_passed() {
    // The tag starts a workflow this app never watches. Saying "released"
    // would be claiming an outcome nothing here observed.
    let text = describe_tag(&TagOutcome::Tagged {
        tag: "v2.4.8".into(),
        remote: "origin".into(),
    });
    assert!(text.contains("Tagged v2.4.8"), "{text}");
    assert!(text.contains("origin"), "{text}");
    assert!(text.contains("release workflow"), "{text}");
    assert!(text.contains("Nothing here checked"), "{text}");
}
