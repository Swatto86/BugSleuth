//! Attempting to prove a finding, and judging the attempt mechanically.
//!
//! The judgement never takes the model's word for anything. Three observations
//! decide it, all made by running tests ourselves:
//!
//! 1. Before the attempt, the suite is green. If it is not, nothing after it
//!    means anything.
//! 2. After the attempt, the model's new test must fail.
//! 3. After the attempt, *every previously passing test must still pass*.
//!
//! Point 3 is the one that is easy to leave out and expensive to omit. A coding
//! agent asked to make a test fail can always succeed by breaking the code — and
//! that is exactly the shape of a false proof: a red test that says nothing
//! about the defect it was supposed to demonstrate.

use std::path::Path;
use std::time::Duration;

use bugsleuth_domain::{ProofClaim, ProofVerdict};
use bugsleuth_provider::claude::{ProveRequest, prove as run_model};
use bugsleuth_verify::{Outcome, Worktree, counts, run_tests};

pub struct Attempt<'a> {
    /// Repository containing the defect. A worktree is made from it; it is never
    /// itself modified.
    pub repo: &'a Path,
    /// Commit to base the worktree on.
    pub commit: &'a str,
    pub model: &'a str,
    /// The defect description given to the model.
    pub brief: &'a str,
    /// Test command, e.g. `cargo test -p alder-infrastructure --lib`.
    pub test_command: &'a str,
    pub max_turns: u32,
    pub timeout: Duration,
    pub test_timeout: Duration,
    pub api_key: Option<&'a str>,
    pub label: &'a str,
}

pub struct AttemptReport {
    pub verdict: ProofVerdict,
    pub claim: Option<ProofClaim>,
    pub baseline_passed: u32,
    pub after_passed: u32,
    pub after_failed: u32,
    pub changed_files: Vec<String>,
    pub turns: Option<u32>,
    /// The model's added test as a patch, so it can be replayed elsewhere —
    /// notably against fixed code, to check the test detects the fix.
    pub patch: String,
    pub detail: String,
}

impl AttemptReport {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "proof attempt: {}\n  {}\n",
            if self.verdict.is_proof() {
                "PROVED"
            } else {
                "NOT PROVED"
            },
            self.verdict.describe()
        ));
        out.push_str(&format!("  {}\n", self.detail));
        out.push_str(&format!(
            "  tests: {} passed before, {} passed and {} failed after\n",
            self.baseline_passed, self.after_passed, self.after_failed
        ));
        if let Some(turns) = self.turns {
            out.push_str(&format!("  turns: {turns}\n"));
        }
        if !self.changed_files.is_empty() {
            out.push_str(&format!("  changed: {}\n", self.changed_files.join(", ")));
        }
        if let Some(claim) = &self.claim {
            out.push_str(&format!(
                "  model said: wrote_failing_test={}, test={}\n",
                claim.wrote_failing_test, claim.test_name
            ));
            if !claim.obstacle.trim().is_empty() {
                out.push_str(&format!("  obstacle: {}\n", claim.obstacle));
            }
        }
        out
    }
}

pub async fn attempt(spec: Attempt<'_>) -> anyhow::Result<AttemptReport> {
    let worktree = Worktree::create(spec.repo, spec.commit, spec.label)?;
    let dir = worktree.path().to_path_buf();

    // 1. Establish that the tree is green before the model touches it.
    let baseline = run_tests(&dir, spec.test_command, None, spec.test_timeout)?;
    let (baseline_passed, baseline_failed) = counts(&baseline.stdout);
    if baseline.outcome != Outcome::Passed || baseline_failed > 0 {
        anyhow::bail!(
            "the baseline test suite is not green ({} passed, {} failed): a proof attempt against \
             a red tree cannot mean anything.\n{}",
            baseline_passed,
            baseline_failed,
            baseline.summary()
        );
    }

    // 2. Let the model try.
    let result = run_model(ProveRequest {
        worktree: &dir,
        model: spec.model,
        brief: spec.brief,
        timeout: spec.timeout,
        max_turns: spec.max_turns,
        binary: None,
        api_key: spec.api_key,
    })
    .await;

    let result = match result {
        Ok(result) => result,
        Err(error) => {
            return Ok(AttemptReport {
                verdict: ProofVerdict::NoTestWritten,
                claim: None,
                baseline_passed,
                after_passed: 0,
                after_failed: 0,
                changed_files: vec![],
                turns: None,
                patch: String::new(),
                detail: format!("the proof attempt did not complete: {error}"),
            });
        }
    };

    let changed_files = worktree.changed_files().unwrap_or_default();
    let patch = capture_patch(&dir);

    // 3. Judge by observation, not by the model's account.
    let (verdict, after_passed, after_failed, detail) =
        judge(&dir, &spec, &result.claim, baseline_passed)?;

    Ok(AttemptReport {
        verdict,
        claim: Some(result.claim),
        baseline_passed,
        after_passed,
        after_failed,
        changed_files,
        turns: result.turns,
        patch,
        detail,
    })
}

fn judge(
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

    match full.outcome {
        Outcome::DidNotBuild => {
            return Ok((
                ProofVerdict::DidNotBuild,
                after_passed,
                after_failed,
                first_error(&full.stderr),
            ));
        }
        Outcome::TimedOut => {
            return Ok((
                ProofVerdict::TimedOut,
                after_passed,
                after_failed,
                "the suite was killed for running too long".to_string(),
            ));
        }
        Outcome::Passed => {
            return Ok((
                ProofVerdict::TestDoesNotFail,
                after_passed,
                after_failed,
                format!(
                    "every test passes, including `{}` — it demonstrates nothing",
                    claim.test_name
                ),
            ));
        }
        Outcome::Failed => {}
    }

    // Something failed. Was it only the new test, or did the model break the code?
    // A test that fails because production code was sabotaged is not evidence
    // about the original defect.
    if after_passed < baseline_passed {
        return Ok((
            ProofVerdict::SuiteSabotaged,
            after_passed,
            after_failed,
            format!(
                "{baseline_passed} tests passed before the attempt and only {after_passed} after, \
                 so production code was changed rather than a test being added"
            ),
        ));
    }

    // Confirm the failure really is the named test, not some unrelated flake.
    let single = run_tests(
        dir,
        spec.test_command,
        Some(claim.test_name.trim()),
        spec.test_timeout,
    )?;
    let (single_passed, single_failed) = counts(&single.stdout);
    if single_passed + single_failed == 0 {
        return Ok((
            ProofVerdict::TestNotFound,
            after_passed,
            after_failed,
            format!("no test matches `{}`", claim.test_name),
        ));
    }
    if single_failed == 0 {
        return Ok((
            ProofVerdict::TestDoesNotFail,
            after_passed,
            after_failed,
            format!(
                "`{}` passes on its own; the failure elsewhere is unrelated",
                claim.test_name
            ),
        ));
    }

    Ok((
        ProofVerdict::Proved,
        after_passed,
        after_failed,
        format!(
            "`{}` fails and all {baseline_passed} previously passing tests still pass",
            claim.test_name
        ),
    ))
}

/// The model's changes as a patch, including files it newly created.
fn capture_patch(dir: &Path) -> String {
    let add = std::process::Command::new("git")
        .args(["add", "-AN"])
        .current_dir(dir)
        .output();
    if add.is_err() {
        return String::new();
    }
    std::process::Command::new("git")
        .args(["diff"])
        .current_dir(dir)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
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
