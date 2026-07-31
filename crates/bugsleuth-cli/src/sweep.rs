//! One lane sweep, end to end: brief the model, run it, verify what comes back.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use bugsleuth_domain::{Finding, FindingId, Lane, ModelId, RawFinding};
use bugsleuth_provider::claude::{self, ClaudeSweep};
use bugsleuth_provider::codex::{self, CodexSweep};
use bugsleuth_verify::verify_anchor;

use crate::brief;
use crate::report::{LaneReport, Rejected, Status, rank};

/// Which CLI to run, and which model within it.
///
/// Dispatch is a plain enum rather than a trait with one implementation per
/// vendor. The set of vendors is closed and small: three CLIs we ship support
/// for ourselves. A trait would buy extensibility nobody needs while making the
/// differences between adapters harder to see. Revisit when a fourth vendor
/// appears and the shape has stopped moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Claude,
    Codex,
}

impl Vendor {
    /// Read a `vendor:model` spec such as `codex:gpt-5.6-codex`. A bare name
    /// means Claude, which keeps the common case short.
    pub fn parse(spec: &str) -> (Vendor, &str) {
        match spec.split_once(':') {
            Some(("codex", model)) => (Vendor::Codex, model),
            Some(("claude", model)) => (Vendor::Claude, model),
            _ => (Vendor::Claude, spec),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Vendor::Claude => "claude",
            Vendor::Codex => "codex",
        }
    }
}

pub struct Request<'a> {
    pub repo: &'a Path,
    pub lane: Lane,
    /// `vendor:model`, or a bare model name for Claude.
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
    let (vendor, model) = Vendor::parse(request.model);
    let model_label = format!("{}:{model}", vendor.label());
    let brief = brief::build(request.lane, request.scope);

    let outcome = match vendor {
        Vendor::Claude => claude::sweep(ClaudeSweep {
            repo: request.repo,
            lane: request.lane,
            model,
            brief: &brief,
            timeout: request.timeout,
            max_turns: request.max_turns,
            binary: None,
            api_key: request.api_key,
        })
        .await
        .map(|r| (r.findings.findings, r.turns)),
        Vendor::Codex => codex::sweep(CodexSweep {
            repo: request.repo,
            model,
            brief: &brief,
            timeout: request.timeout,
            binary: None,
        })
        .await
        .map(|r| (r.findings.findings, None)),
    };

    let (raw, turns) = match outcome {
        Ok(outcome) => outcome,
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

    let (findings, rejected) =
        verify_all(request.repo, request.lane, &ModelId::new(&model_label), raw);

    LaneReport {
        lane: request.lane.title().to_string(),
        model: model_label,
        status: Status::Swept { turns },
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
    let (claude, codex) = tokio::join!(claude::probe(), codex::probe());
    let mut usable = 0;
    for (name, probe) in [("claude", claude), ("codex", codex)] {
        match probe {
            Ok(version) => {
                println!("{name}: OK ({version})");
                usable += 1;
            }
            Err(error) => println!("{name}: UNAVAILABLE - {error}"),
        }
    }
    println!(
        "
{usable} of 2 provider CLIs can be started."
    );
    println!("This does not prove they are signed in; only a real sweep does that.");
    if usable == 0 {
        std::process::exit(2);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_model_name_means_claude_so_the_common_case_stays_short() {
        assert_eq!(Vendor::parse("sonnet"), (Vendor::Claude, "sonnet"));
    }

    #[test]
    fn a_vendor_prefix_selects_that_vendor() {
        assert_eq!(
            Vendor::parse("codex:gpt-5.6-codex"),
            (Vendor::Codex, "gpt-5.6-codex")
        );
        assert_eq!(Vendor::parse("claude:opus"), (Vendor::Claude, "opus"));
    }

    #[test]
    fn an_unknown_prefix_is_treated_as_a_model_name_not_silently_dropped() {
        // Model ids legitimately contain colons, so an unrecognised prefix must
        // not be swallowed as a vendor.
        assert_eq!(
            Vendor::parse("anthropic:claude-opus-5"),
            (Vendor::Claude, "anthropic:claude-opus-5")
        );
    }
}
