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

/// Try to prove the top `top` defects, in rank order.
pub async fn prove_top(ranked: &[Ranked], options: &ProveOptions<'_>) -> Vec<Proved> {
    let mut results = Vec::new();

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
            // It must not be reported as though the defect were disproved.
            Err(error) => (
                ProofVerdict::NoTestWritten,
                format!("the proof attempt could not be run: {error}"),
            ),
        };

        eprintln!(
            "proof {}/{}: {} - {}",
            entry.position,
            options.top,
            if verdict.is_proof() {
                "PROVED"
            } else {
                "not proved"
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
    let unproven: Vec<&Proved> = results.iter().filter(|r| !r.verdict.is_proof()).collect();

    out.push_str(&format!(
        "  attempted the top {} of {ranked_total} defects: {} proved, {} not\n",
        results.len(),
        proven.len(),
        unproven.len()
    ));

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
}
