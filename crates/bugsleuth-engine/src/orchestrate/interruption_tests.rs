//! Tests for a run that gives up before it has swept everything.
//!
//! Split from the run tests at the hard line cap, on its own subject: those are
//! about a run reaching its end, and these are about one deciding not to. The
//! distinction matters to the user more than it looks — a lane that was never
//! attempted has not been paid for and will be swept next time, and a report
//! that presents it as a lane that failed sends someone looking at their own
//! code for a problem that is not there.

/// Stopping is for a spent allowance, not for one unlucky sweep.
///
/// The rule this checks decides whether a run gives up. Too eager and a single
/// transient failure abandons a run the user is paying for; too reluctant and a
/// spent allowance is discovered again once per remaining unit, each with its
/// own wait and its own retry.
#[test]
fn a_run_stops_only_when_a_whole_batch_was_refused_and_work_is_left() {
    use super::batch::allowance_spent;

    // Every sweep in the batch refused, and there is more to do.
    assert!(allowance_spent(3, 3, 5));
    assert!(allowance_spent(1, 1, 1));

    // Something got through, so the allowance is not spent.
    assert!(!allowance_spent(3, 2, 5));
    assert!(!allowance_spent(3, 0, 5));

    // Nothing left to attempt: the run reached its end, so there is nothing to
    // stop and nothing to resume.
    assert!(!allowance_spent(3, 3, 0));

    // An empty batch says nothing either way and must not end the run.
    assert!(!allowance_spent(0, 0, 5));
}

/// An interrupted run names every lane it never attempted, and says what to do.
#[test]
fn an_interrupted_run_names_what_it_never_attempted() {
    use crate::plan::Unit;

    let remaining = vec![Unit {
        model: "sonnet".to_string(),
        lane: bugsleuth_domain::Lane::Security,
        pass: 1,
        effort: String::new(),
        use_agents: false,
    }];
    let mut gaps = Vec::new();
    super::gaps::note_interrupted(Some("rate limit reached"), &remaining, &mut gaps);
    assert_eq!(gaps.len(), 1);
    let reason = &gaps[0].reason;
    // The cause, so the reader knows it was not their code.
    assert!(reason.contains("rate limit reached"), "{reason}");
    // And that the sweeps already paid for are not lost, which is the whole
    // difference between this and a run that simply failed.
    assert!(reason.contains("already swept is saved"), "{reason}");

    // A run that was not interrupted says nothing, rather than adding an empty
    // gap that would read as a lane nobody assigned.
    let mut none = Vec::new();
    super::gaps::note_interrupted(None, &remaining, &mut none);
    assert!(none.is_empty());
}
