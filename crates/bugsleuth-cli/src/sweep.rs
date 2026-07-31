//! One lane sweep, end to end: brief the model, run it, verify what comes back.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use bugsleuth_domain::{Finding, FindingId, Lane, ModelId, RawFinding};
use bugsleuth_provider::claude::{self, ClaudeSweep};
use bugsleuth_verify::verify_anchor;

use crate::brief;
use crate::report::{LaneReport, Rejected, Status, rank};

pub struct Request<'a> {
    pub repo: &'a Path,
    pub lane: Lane,
    pub model: &'a str,
    pub scope: Option<&'a str>,
    pub max_turns: u32,
    pub timeout: Duration,
    pub api_key: Option<&'a str>,
}

/// Run the sweep. Never returns an error for a failed sweep — a failure is a
/// *reported state*, because the one outcome this tool must never produce is a
/// lane that quietly looks clean when it never ran.
pub async fn run(request: Request<'_>) -> LaneReport {
    let model_label = format!("claude:{}", request.model);
    let brief = brief::build(request.lane, request.scope);

    let result = claude::sweep(ClaudeSweep {
        repo: request.repo,
        lane: request.lane,
        model: request.model,
        brief: &brief,
        timeout: request.timeout,
        max_turns: request.max_turns,
        binary: None,
        api_key: request.api_key,
    })
    .await;

    let result = match result {
        Ok(result) => result,
        Err(error) => {
            return LaneReport {
                lane: request.lane.title().to_string(),
                model: model_label,
                status: Status::NotSwept {
                    reason: error.to_string(),
                },
                findings: vec![],
                rejected: vec![],
            };
        }
    };

    let (findings, rejected) = verify_all(
        request.repo,
        request.lane,
        &ModelId::new(&model_label),
        result.findings.findings,
    );

    LaneReport {
        lane: request.lane.title().to_string(),
        model: model_label,
        status: Status::Swept {
            turns: result.turns,
        },
        findings,
        rejected,
    }
}

/// Split reported findings into those whose quoted code was located in the file
/// they name, and those that were not.
fn verify_all(
    repo: &Path,
    lane: Lane,
    model: &ModelId,
    raw: Vec<RawFinding>,
) -> (Vec<Finding>, Vec<Rejected>) {
    let mut verified = Vec::new();
    let mut rejected = Vec::new();

    for (index, finding) in raw.into_iter().enumerate() {
        // A lane must not report against files it has no mandate over — this is
        // what stops the UX lane filing "no loading state" on a backend module.
        if !lane.covers(&finding.file) {
            rejected.push(Rejected {
                title: finding.title,
                claimed_file: finding.file,
                claimed_line: finding.line,
                reason: format!("outside the {} lane's file scope", lane.title()),
            });
            continue;
        }

        match verify_anchor(repo, &finding) {
            Ok(anchor) => {
                let id = FindingId::new(format!("{}-{index}", lane.slug()));
                verified.push(Finding::new(id, lane, model.clone(), finding, anchor));
            }
            Err(reason) => rejected.push(Rejected {
                title: finding.title,
                claimed_file: finding.file,
                claimed_line: finding.line,
                reason: reason.to_string(),
            }),
        }
    }

    rank(&mut verified);
    (verified, rejected)
}

/// Confirm the provider CLI can actually be started before a run commits to it.
///
/// Eir has no equivalent: it discovers a missing or signed-out CLI only when a
/// real call fails. For a sweep that is worth avoiding, because the failure
/// would otherwise arrive after the user has waited for several lanes.
pub async fn preflight() -> Result<()> {
    let probe = claude::probe().await;
    match probe {
        Ok(version) => {
            println!("claude CLI: OK ({version})");
            Ok(())
        }
        Err(error) => {
            println!("claude CLI: UNAVAILABLE — {error}");
            std::process::exit(2);
        }
    }
}
