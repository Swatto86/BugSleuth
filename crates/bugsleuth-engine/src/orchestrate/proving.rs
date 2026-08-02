//! Attempting to prove the defects a run surfaced.
//!
//! This is what closes the loop. Without it BugSleuth produces a ranked list of
//! things models *believe*; with it, the top of that list arrives with evidence
//! a machine checked.
//!
//! Proof is expensive — one model invocation and a full test run each — so only
//! the top of the ranked list is attempted, and how far down is the caller's
//! choice. Everything below that line is reported as **unattempted**, never as
//! unproven: "we did not try" and "we tried and failed" are different facts, and
//! conflating them would misrepresent the weaker one as the stronger.

use std::path::Path;
use std::time::Duration;

use bugsleuth_domain::ProofVerdict;
use bugsleuth_judge::Ranked;

use crate::brief;
use crate::prove::{self, Attempt};

pub struct ProveOptions<'a> {
    pub repo: &'a Path,
    /// Model used for proof attempts, `vendor:model`.
    pub model: &'a str,
    /// Command that runs the tests, e.g. "cargo test".
    pub test_command: &'a str,
    /// How far down the ranked list to attempt. 0 disables proving entirely.
    pub top: usize,
    pub max_turns: u32,
    pub timeout: Duration,
    pub test_timeout: Duration,
    pub api_key: Option<&'a str>,
}

pub struct Proved {
    pub position: usize,
    pub title: String,
    pub verdict: ProofVerdict,
    pub detail: String,
}

/// Whether the repository has uncommitted changes.
///
/// This matters more than it looks. A sweep reads the **working tree**, but a
/// proof attempt runs in a worktree checked out at **HEAD**. If those differ, a
/// defect found in uncommitted code simply is not present in the tree the prover
/// sees, so its test cannot fail — and the run would report a real defect as
/// "the test written for it passes, so it proves nothing".
///
/// That is precisely the kind of confidently-wrong output this tool exists to
/// avoid, so the mismatch is surfaced rather than left to be misread.
pub fn has_uncommitted_changes(repo: &Path) -> bool {
    bugsleuth_verify::hide_console_window(&mut std::process::Command::new("git"))
        .args(["status", "--porcelain"])
        .current_dir(repo)
        .output()
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(false)
}

/// Try to prove the top `top` defects, in rank order.
pub async fn prove_top(ranked: &[Ranked], options: &ProveOptions<'_>) -> Vec<Proved> {
    let mut results = Vec::new();

    if !ranked.is_empty() && options.top > 0 && has_uncommitted_changes(options.repo) {
        eprintln!(
            "warning: the repository has uncommitted changes. Sweeps read the working\n         \
             tree, but proof attempts run against HEAD, so a defect in uncommitted\n         \
             code cannot be proved and will be reported as not proved. Commit or\n         \
             stash before proving."
        );
    }

    for entry in ranked.iter().take(options.top) {
        let finding = entry.cluster.representative();
        // Give the prover the same evidence the report shows, so it is arguing
        // about the defect that was actually reported rather than re-deriving one.
        let defect = format!(
            "{}\n\nLocation: {}:{}\n\n{}\n\nWhen it goes wrong: {}\n\nThe code in question:\n{}",
            finding.title,
            finding.anchor.file,
            finding.anchor.line,
            finding.explanation,
            finding.failure_scenario,
            finding.anchor.snippet,
        );

        let attempt = prove::attempt(Attempt {
            repo: options.repo,
            commit: "HEAD",
            model: options.model,
            brief: &brief::proof(&defect, options.test_command),
            test_command: options.test_command,
            max_turns: options.max_turns,
            timeout: options.timeout,
            test_timeout: options.test_timeout,
            api_key: options.api_key,
            // One worktree per rank position, so two attempts never collide.
            label: &format!("proof-{}", entry.position),
        })
        .await;

        let (verdict, detail) = match attempt {
            Ok(report) => (report.verdict, report.detail),
            // A proof attempt that could not run says nothing about the defect.
            // It must not be reported as though the defect were disproved — and
            // for a while it was, because `NoTestWritten` is bucketed with the
            // genuine failures and explained as a defect that resisted a test.
            Err(error) => (
                ProofVerdict::NotAttempted,
                format!("the proof attempt could not be run: {error}"),
            ),
        };

        eprintln!(
            "proof {}/{}: {} - {}",
            entry.position,
            options.top,
            match verdict {
                ProofVerdict::Proved => "PROVED",
                // The live line has to make the same distinction the report
                // does, or someone watching a run sees twenty "not proved"
                // lines and concludes the findings were weak when the CLI was
                // simply not there.
                ProofVerdict::NotAttempted => "could not attempt",
                _ => "not proved",
            },
            finding.title
        );

        results.push(Proved {
            position: entry.position,
            title: finding.title.clone(),
            verdict,
            detail,
        });
    }

    results
}

/// Render the proof outcomes, keeping proven and unproven strictly apart.
pub fn to_text(results: &[Proved], ranked_total: usize) -> String {
    if results.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n=== proof ===\n");
    let proven: Vec<&Proved> = results.iter().filter(|r| r.verdict.is_proof()).collect();
    // An attempt that never ran is not evidence about the defect, and saying so
    // in the same breath as "this resisted a test" told the reader the opposite
    // of the truth: nothing had been tried. A missing CLI became "may need
    // concurrency, I/O or timing to reproduce".
    let unattempted: Vec<&Proved> = results
        .iter()
        .filter(|r| r.verdict == ProofVerdict::NotAttempted)
        .collect();
    let unproven: Vec<&Proved> = results
        .iter()
        .filter(|r| !r.verdict.is_proof() && r.verdict != ProofVerdict::NotAttempted)
        .collect();

    out.push_str(&format!(
        "  attempted the top {} of {ranked_total} defects: {} proved, {} not, \
         {} could not be attempted\n",
        results.len(),
        proven.len(),
        unproven.len(),
        unattempted.len()
    ));

    if !unattempted.is_empty() {
        out.push_str(
            "\n  COULD NOT BE ATTEMPTED - the attempt itself failed to run, so these say\n  \
             nothing either way about the defect. Usually a missing or signed-out CLI,\n  \
             or a rate limit:\n",
        );
        for entry in &unattempted {
            out.push_str(&format!(
                "    #{} {}\n       {}\n",
                entry.position, entry.title, entry.detail
            ));
        }
    }

    if !proven.is_empty() {
        out.push_str(
            "\n  PROVED - a test was written that fails because of this, and every\n  \
                      previously passing test still passes:\n",
        );
        for entry in &proven {
            out.push_str(&format!("    #{} {}\n", entry.position, entry.title));
        }
    }

    if !unproven.is_empty() {
        out.push_str(
            "\n  NOT PROVED - these may still be real. A defect that resists a test is\n  \
             often one that needs concurrency, I/O or timing to reproduce:\n",
        );
        for entry in &unproven {
            out.push_str(&format!(
                "    #{} {}\n       {}\n",
                entry.position, entry.title, entry.detail
            ));
        }
    }

    if ranked_total > results.len() {
        out.push_str(&format!(
            "\n  {} further defects were NOT attempted - no proof was tried for them,\n  \
             which is not the same as failing to prove them.\n",
            ranked_total - results.len()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proved(position: usize, verdict: ProofVerdict) -> Proved {
        Proved {
            position,
            title: format!("defect {position}"),
            verdict,
            detail: "detail".into(),
        }
    }

    #[test]
    fn proven_and_unproven_findings_are_never_mixed_together() {
        let text = to_text(
            &[
                proved(1, ProofVerdict::Proved),
                proved(2, ProofVerdict::TestDoesNotFail),
            ],
            2,
        );
        let proved_at = text.find("PROVED -").unwrap_or(usize::MAX);
        let unproved_at = text.find("NOT PROVED").unwrap_or(0);
        assert!(proved_at < unproved_at, "the two sections ran together");
        assert!(text.contains("1 proved, 1 not"));
    }

    #[test]
    fn defects_below_the_cut_are_reported_as_unattempted_not_unproven() {
        let text = to_text(&[proved(1, ProofVerdict::Proved)], 9);
        assert!(text.contains("8 further defects were NOT attempted"));
        assert!(text.contains("not the same as failing to prove them"));
    }

    #[test]
    fn a_failed_proof_is_not_presented_as_the_defect_being_disproved() {
        let text = to_text(&[proved(1, ProofVerdict::NoTestWritten)], 1);
        assert!(text.contains("may still be real"));
    }

    #[test]
    fn proving_nothing_renders_nothing_rather_than_an_empty_heading() {
        assert_eq!(to_text(&[], 5), "");
    }

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
            !has_uncommitted_changes(&dir),
            "a freshly committed tree is clean"
        );

        let _ = std::fs::write(dir.join("a.txt"), "changed\n");
        assert!(
            has_uncommitted_changes(&dir),
            "an edited working tree is dirty, and proving would silently miss it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_attempt_that_never_ran_is_not_reported_as_a_defect_resisting_a_test() {
        // A missing or signed-out CLI used to land under "NOT PROVED - a defect
        // that resists a test is often one that needs concurrency, I/O or
        // timing to reproduce". That told the reader the defect was hard to
        // reproduce when in truth nothing had been tried at all.
        let text = to_text(&[proved(1, ProofVerdict::NotAttempted)], 1);
        assert!(text.contains("COULD NOT BE ATTEMPTED"), "{text}");
        assert!(text.contains("nothing either way"), "{text}");
        assert!(
            !text.contains("resists a test"),
            "an unrun attempt is still described as a stubborn defect: {text}"
        );
    }

    #[test]
    fn a_genuine_failure_to_prove_still_reads_as_one() {
        // The distinction has to cut both ways: a test that was written and
        // demonstrated nothing is a result, and must not be excused as a
        // failure to run.
        let text = to_text(&[proved(1, ProofVerdict::TestDoesNotFail)], 1);
        assert!(text.contains("NOT PROVED"), "{text}");
        assert!(!text.contains("COULD NOT BE ATTEMPTED"), "{text}");
    }
}
