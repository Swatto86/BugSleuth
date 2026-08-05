//! Deciding what a proof attempt actually demonstrated.
//!
//! The decision is split from the test running so the rules can be tested
//! without a repository, a model, or a cargo build. These few lines are the
//! most trust-critical logic in the tool: they are what stands between "a model
//! asserted something" and "a machine confirmed it".

use std::collections::BTreeSet;
use std::path::Path;

use bugsleuth_domain::{ProofClaim, ProofVerdict};
use bugsleuth_verify::{Outcome, counts, run_tests};

use super::Attempt;

pub(super) fn judge(
    dir: &Path,
    spec: &Attempt<'_>,
    claim: &ProofClaim,
    baseline_passed: u32,
    baseline_tests: &BTreeSet<String>,
) -> anyhow::Result<(ProofVerdict, u32, u32, String)> {
    if !claim.wrote_failing_test || claim.test_name.trim().is_empty() {
        let obstacle = if claim.obstacle.trim().is_empty() {
            "the model reported no test and gave no reason".to_string()
        } else {
            claim.obstacle.clone()
        };
        return Ok((ProofVerdict::NoTestWritten, 0, 0, obstacle));
    }

    let full = run_tests(dir, spec.test_command, None, spec.test_timeout)?;
    let (after_passed, after_failed) = counts(&full.stdout);

    // Decide the outcomes the whole-suite run settles on its own — those need no
    // per-test identities. A red suite (`Failed`) falls through to the checks
    // below.
    if let Some(verdict) = from_suite(&full.outcome) {
        let detail = match verdict {
            ProofVerdict::DidNotBuild => first_error(&full.stderr),
            ProofVerdict::TimedOut => "the suite was killed for running too long".to_string(),
            ProofVerdict::TestDoesNotFail => format!(
                "every test passes, including `{}` — it demonstrates nothing",
                claim.test_name
            ),
            _ => String::new(),
        };
        return Ok((verdict, after_passed, after_failed, detail));
    }

    // The suite is red. Before that redness can be trusted as evidence about the
    // defect, prove that every test which passed at the baseline still passes.
    // A count cannot: an attempt that breaks one existing test and adds one
    // passing test leaves the total unchanged, so only the set of names shows
    // the loss. This is the single most important check in the tool.
    if let Some((verdict, detail)) =
        sabotage_check(baseline_passed, baseline_tests, &full.passed_tests)
    {
        return Ok((verdict, after_passed, after_failed, detail));
    }

    // Confirm the failure really is the named test, not something unrelated.
    let single = run_tests(
        dir,
        spec.test_command,
        Some(claim.test_name.trim()),
        spec.test_timeout,
    )?;
    // The run's own outcome first. It was computed and never read, so a
    // confirmation that timed out or failed to build produced no test counts,
    // fell through to `passed + failed == 0`, and was reported as "no test
    // matches that name" — telling the reader the model had invented a test
    // when in fact the run never got far enough to look for one.
    if let Some(verdict) = from_confirmation(&single.outcome) {
        let detail = match verdict {
            ProofVerdict::DidNotBuild => first_error(&single.stderr),
            _ => format!(
                "the run of `{}` on its own was killed for taking too long, so                  nothing was settled either way",
                claim.test_name
            ),
        };
        return Ok((verdict, after_passed, after_failed, detail));
    }

    let (single_passed, single_failed) = counts(&single.stdout);
    let verdict = from_named_test(single_passed, single_failed);
    let detail = match verdict {
        ProofVerdict::TestNotFound => format!("no test matches `{}`", claim.test_name),
        ProofVerdict::TestDoesNotFail => format!(
            "`{}` passes on its own; the failure elsewhere is unrelated",
            claim.test_name
        ),
        _ => format!(
            "`{}` fails and all {baseline_passed} previously passing tests still pass",
            claim.test_name
        ),
    };
    Ok((verdict, after_passed, after_failed, detail))
}

/// What the whole-suite run's outcome alone settles, before any test identities
/// are consulted. `None` means the suite failed, which is promising but still
/// has to survive the sabotage check and then be confirmed to be the named test.
fn from_suite(outcome: &Outcome) -> Option<ProofVerdict> {
    match outcome {
        Outcome::DidNotBuild => Some(ProofVerdict::DidNotBuild),
        Outcome::TimedOut => Some(ProofVerdict::TimedOut),
        Outcome::Passed => Some(ProofVerdict::TestDoesNotFail),
        Outcome::Failed => None,
    }
}

/// Whether a red suite lost any test that passed at the baseline — or whether
/// that cannot even be established. `None` means every baseline test still
/// passes, so the redness is a candidate proof.
///
/// The sabotage rule, and the single most important check in the tool: a coding
/// agent asked to make a test fail can always succeed by breaking the code, so a
/// red suite is only evidence about the original defect if nothing that used to
/// pass has stopped. Identities decide it, not counts — a broken existing test
/// hidden behind a newly added passing one keeps the count identical.
fn sabotage_check(
    baseline_passed: u32,
    baseline_tests: &BTreeSet<String>,
    after_tests: &BTreeSet<String>,
) -> Option<(ProofVerdict, String)> {
    // A runner that reported passing tests at the baseline but whose output we
    // cannot read individual test names from cannot support the "every previous
    // test still passes" claim. Fail closed rather than fall back to counting.
    if baseline_passed > 0 && baseline_tests.is_empty() {
        return Some((
            ProofVerdict::SuiteSabotaged,
            "the baseline passed tests but their names could not be read from the runner \
             output, so it cannot be shown that every previously passing test still passes"
                .to_string(),
        ));
    }
    if !baseline_tests.is_subset(after_tests) {
        let missing: Vec<String> = baseline_tests.difference(after_tests).cloned().collect();
        return Some((
            ProofVerdict::SuiteSabotaged,
            format!(
                "previously passing tests no longer pass: {}",
                missing.join(", ")
            ),
        ));
    }
    None
}

/// What the confirmation run settles before its counts are even looked at.
///
/// Deliberately separate from [`from_suite`]: that one also decides
/// `TestDoesNotFail` and `SuiteSabotaged` from a comparison against the
/// baseline, and neither applies to a single test run on its own.
fn from_confirmation(outcome: &Outcome) -> Option<ProofVerdict> {
    match outcome {
        Outcome::DidNotBuild => Some(ProofVerdict::DidNotBuild),
        Outcome::TimedOut => Some(ProofVerdict::TimedOut),
        Outcome::Passed | Outcome::Failed => None,
    }
}

/// What running only the model's named test settles.
fn from_named_test(passed: u32, failed: u32) -> ProofVerdict {
    if passed + failed == 0 {
        ProofVerdict::TestNotFound
    } else if failed == 0 {
        ProofVerdict::TestDoesNotFail
    } else {
        ProofVerdict::Proved
    }
}

fn first_error(stderr: &str) -> String {
    stderr
        .lines()
        .find(|line| line.contains("error"))
        .unwrap_or("the tree does not compile")
        .chars()
        .take(300)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a set of test names from string literals, for the identity checks.
    fn names(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn breaking_existing_tests_is_never_accepted_as_proof() {
        // The model made its test fail by sabotaging production code: two tests
        // that used to pass no longer appear in the after set.
        let (verdict, _) =
            sabotage_check(3, &names(&["old_a", "old_b", "old_c"]), &names(&["old_a"]))
                .expect("a lost test must be caught");
        assert_eq!(verdict, ProofVerdict::SuiteSabotaged);
    }

    #[test]
    fn previously_passing_test_identity_cannot_be_replaced_by_a_new_pass() {
        // The count is identical — two before, two after — but one of the
        // originals was replaced by a brand-new passing test. A count-only check
        // sees no loss; the identity check must.
        let baseline = names(&["old_a", "old_b"]);
        let after = names(&["old_a", "newly_added"]);
        assert_eq!(baseline.len(), after.len());
        let (verdict, _) =
            sabotage_check(2, &baseline, &after).expect("a replaced test must be caught");
        assert_eq!(verdict, ProofVerdict::SuiteSabotaged);
    }

    #[test]
    fn a_runner_that_hides_its_test_names_cannot_prove_preservation() {
        // Passing tests at the baseline, but no names to compare against: fail
        // closed rather than fall back to a count.
        let (verdict, _) = sabotage_check(50, &BTreeSet::new(), &names(&["something"]))
            .expect("unreadable names must not be treated as no loss");
        assert_eq!(verdict, ProofVerdict::SuiteSabotaged);
    }

    #[test]
    fn a_red_suite_that_kept_every_passing_test_needs_the_named_test_checked() {
        // Every baseline test still passes and something new fails — the good
        // case, but not yet confirmed to be the model's test.
        assert_eq!(from_suite(&Outcome::Failed), None);
        assert_eq!(
            sabotage_check(2, &names(&["old_a", "old_b"]), &names(&["old_a", "old_b"])),
            None
        );
    }

    #[test]
    fn a_green_suite_proves_nothing() {
        assert_eq!(
            from_suite(&Outcome::Passed),
            Some(ProofVerdict::TestDoesNotFail)
        );
    }

    #[test]
    fn a_broken_build_is_not_a_demonstrated_defect() {
        assert_eq!(
            from_suite(&Outcome::DidNotBuild),
            Some(ProofVerdict::DidNotBuild)
        );
        assert_eq!(from_suite(&Outcome::TimedOut), Some(ProofVerdict::TimedOut));
    }

    #[test]
    fn only_a_named_test_that_actually_ran_and_failed_is_proof() {
        assert_eq!(from_named_test(0, 1), ProofVerdict::Proved);
        assert_eq!(from_named_test(0, 0), ProofVerdict::TestNotFound);
        assert_eq!(from_named_test(1, 0), ProofVerdict::TestDoesNotFail);
    }
}
