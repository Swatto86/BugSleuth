//! Deciding what a proof attempt actually demonstrated.
//!
//! The decision is split from the test running so the rules can be tested
//! without a repository, a model, or a cargo build. These few lines are the
//! most trust-critical logic in the tool: they are what stands between "a model
//! asserted something" and "a machine confirmed it".

use std::path::Path;

use bugsleuth_domain::{ProofClaim, ProofVerdict};
use bugsleuth_verify::{Outcome, counts, run_tests};

use super::Attempt;

pub(super) fn judge(
    dir: &Path,
    spec: &Attempt<'_>,
    claim: &ProofClaim,
    baseline_passed: u32,
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

    // Decide as far as the whole-suite run allows. Only a suite that failed
    // *without* losing any previously passing test needs a second look.
    if let Some(verdict) = from_suite(&full.outcome, baseline_passed, after_passed) {
        let detail = match verdict {
            ProofVerdict::DidNotBuild => first_error(&full.stderr),
            ProofVerdict::TimedOut => "the suite was killed for running too long".to_string(),
            ProofVerdict::TestDoesNotFail => format!(
                "every test passes, including `{}` — it demonstrates nothing",
                claim.test_name
            ),
            ProofVerdict::SuiteSabotaged => format!(
                "{baseline_passed} tests passed before the attempt and only {after_passed} after, \
                 so production code was changed rather than a test being added"
            ),
            _ => String::new(),
        };
        return Ok((verdict, after_passed, after_failed, detail));
    }

    // Confirm the failure really is the named test, not something unrelated.
    let single = run_tests(
        dir,
        spec.test_command,
        Some(claim.test_name.trim()),
        spec.test_timeout,
    )?;
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

/// What the whole-suite run alone settles. `None` means "a red suite that did
/// not lose any previously passing test" — promising, but it still has to be
/// confirmed that the redness is the model's named test.
///
/// The sabotage rule lives here: if fewer tests pass after the attempt than
/// before it, the model changed production code rather than only adding a test.
/// Its new failing test is then not evidence about the original defect, because
/// a coding agent asked to make a test fail can always succeed by breaking the
/// code. This is the single most important check in the tool.
fn from_suite(outcome: &Outcome, baseline_passed: u32, after_passed: u32) -> Option<ProofVerdict> {
    match outcome {
        Outcome::DidNotBuild => Some(ProofVerdict::DidNotBuild),
        Outcome::TimedOut => Some(ProofVerdict::TimedOut),
        Outcome::Passed => Some(ProofVerdict::TestDoesNotFail),
        Outcome::Failed if after_passed < baseline_passed => Some(ProofVerdict::SuiteSabotaged),
        Outcome::Failed => None,
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

    #[test]
    fn breaking_existing_tests_is_never_accepted_as_proof() {
        // The model made its test fail by sabotaging production code: the suite
        // is red, but two tests that used to pass no longer do.
        assert_eq!(
            from_suite(&Outcome::Failed, 50, 48),
            Some(ProofVerdict::SuiteSabotaged)
        );
    }

    #[test]
    fn a_red_suite_that_kept_every_passing_test_needs_the_named_test_checked() {
        // 50 still pass and something new fails — the good case, but not yet
        // confirmed to be the model's test rather than an unrelated failure.
        assert_eq!(from_suite(&Outcome::Failed, 50, 50), None);
    }

    #[test]
    fn a_green_suite_proves_nothing() {
        assert_eq!(
            from_suite(&Outcome::Passed, 50, 51),
            Some(ProofVerdict::TestDoesNotFail)
        );
    }

    #[test]
    fn a_broken_build_is_not_a_demonstrated_defect() {
        assert_eq!(
            from_suite(&Outcome::DidNotBuild, 50, 0),
            Some(ProofVerdict::DidNotBuild)
        );
        assert_eq!(
            from_suite(&Outcome::TimedOut, 50, 0),
            Some(ProofVerdict::TimedOut)
        );
    }

    #[test]
    fn only_a_named_test_that_actually_ran_and_failed_is_proof() {
        assert_eq!(from_named_test(0, 1), ProofVerdict::Proved);
        assert_eq!(from_named_test(0, 0), ProofVerdict::TestNotFound);
        assert_eq!(from_named_test(1, 0), ProofVerdict::TestDoesNotFail);
    }
}
