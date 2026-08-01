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
use bugsleuth_provider::claude::{ProveRequest, prove as claude_prove};
use bugsleuth_provider::codex;
use bugsleuth_verify::{Outcome, Worktree, counts, run_tests};

mod judge;
use judge::judge;

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

/// The model's own account of a proof attempt, before anything is believed.
struct ProveOutcome {
    claim: ProofClaim,
    turns: Option<u32>,
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

    // 2. Let the model try. Which vendor is chosen the same way sweeps choose.
    let (vendor, model) = crate::sweep::Vendor::parse(spec.model);
    let attempt = match vendor {
        crate::sweep::Vendor::Claude => claude_prove(ProveRequest {
            worktree: &dir,
            model,
            brief: spec.brief,
            timeout: spec.timeout,
            max_turns: spec.max_turns,
            binary: None,
            api_key: spec.api_key,
        })
        .await
        .map(|r| (r.claim, r.turns)),
        crate::sweep::Vendor::Codex => codex::prove(&dir, model, spec.brief, spec.timeout)
            .await
            .map(|claim| (claim, None)),
        // Kilo cannot be constrained to a schema and has no per-invocation
        // permission control. Refused rather than attempted: a proof step that
        // cannot be trusted to report accurately is worse than no proof step.
        crate::sweep::Vendor::Kilo => {
            return Ok(AttemptReport {
                verdict: ProofVerdict::NoTestWritten,
                claim: None,
                baseline_passed,
                after_passed: 0,
                after_failed: 0,
                changed_files: vec![],
                turns: None,
                patch: String::new(),
                detail: "kilo cannot be used for proof attempts: it cannot be given an output                          schema to enforce, so its report of what it did cannot be relied on"
                    .to_string(),
            });
        }
    };

    let result = match attempt {
        Ok((claim, turns)) => ProveOutcome { claim, turns },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_non_proved_verdict_reports_as_not_proved() {
        for verdict in [
            ProofVerdict::SuiteSabotaged,
            ProofVerdict::TestDoesNotFail,
            ProofVerdict::TestNotFound,
            ProofVerdict::DidNotBuild,
            ProofVerdict::TimedOut,
            ProofVerdict::NoTestWritten,
        ] {
            let report = AttemptReport {
                verdict,
                claim: None,
                baseline_passed: 50,
                after_passed: 0,
                after_failed: 0,
                changed_files: vec![],
                turns: None,
                patch: String::new(),
                detail: String::new(),
            };
            assert!(report.to_text().contains("NOT PROVED"), "{verdict:?}");
        }
    }
}
