//! One lane sweep, end to end: brief the model, run it, verify what comes back.

mod isolate;

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use bugsleuth_domain::{Finding, FindingId, Lane, ModelId, RawFinding};
use bugsleuth_provider::claude::{self, ClaudeSweep};
use bugsleuth_provider::codex::{self, CodexSweep};
use bugsleuth_provider::kilo::{self, KiloSweep};
use bugsleuth_verify::{Worktree, verify_anchor};

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
    Kilo,
}

impl Vendor {
    /// Read a `vendor:model` spec such as `codex:gpt-5.6-codex`. A bare name
    /// means Claude, which keeps the common case short.
    pub fn parse(spec: &str) -> (Vendor, &str) {
        match spec.split_once(':') {
            Some(("codex", model)) => (Vendor::Codex, model),
            Some(("claude", model)) => (Vendor::Claude, model),
            Some(("kilo", model)) => (Vendor::Kilo, model),
            _ => (Vendor::Claude, spec),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Vendor::Claude => "claude",
            Vendor::Codex => "codex",
            Vendor::Kilo => "kilo",
        }
    }

    /// Whether the CLI can be handed a JSON Schema it will actually enforce.
    /// Kilo cannot, so its brief has to describe the shape in words instead.
    pub fn enforces_schema(self) -> bool {
        !matches!(self, Vendor::Kilo)
    }

    /// Whether a sweep by this vendor must run in a throwaway checkout rather
    /// than against the repository itself.
    ///
    /// True only for Kilo, and not by preference. Codex takes `--sandbox
    /// read-only` and Claude takes a tool allowlist, so neither can write. Kilo
    /// has no per-invocation equivalent — its permissions come from the user's
    /// own global config — so the only way to guarantee a review cannot modify
    /// the code it is reviewing is to give it a copy.
    pub fn needs_isolation(self) -> bool {
        matches!(self, Vendor::Kilo)
    }
}

pub struct Request<'a> {
    pub repo: &'a Path,
    pub lane: Lane,
    /// `vendor:model`, or a bare model name for Claude.
    pub model: &'a str,
    pub scope: Option<&'a str>,
    /// Reasoning effort. Empty means the vendor's own default.
    pub effort: &'a str,
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
    let brief = brief::build(request.lane, request.scope, vendor.enforces_schema());
    // Recorded before anything runs, so even a failed sweep says what tree it
    // was pointed at.
    let commit = reviewed_commit(request.repo);

    let not_swept = |reason: String| LaneReport {
        lane: request.lane.title().to_string(),
        model: model_label.clone(),
        commit: commit.clone(),
        scope: request.scope.map(str::to_string),
        status: Status::NotSwept { reason },
        findings: vec![],
        rejected: vec![],
    };

    // A vendor that cannot be run read-only gets a throwaway checkout instead.
    // The worktree is held for the whole sweep and deletes itself on drop.
    let isolation = if vendor.needs_isolation() {
        match Worktree::create(request.repo, "HEAD", &format!("sweep-{}", vendor.label())) {
            Ok(worktree) => {
                // The worktree is ours and about to be deleted, so the reviewed
                // repository's standing orders can simply be taken out of it.
                // Claude and Codex get the same isolation from a flag; Kilo has
                // none, and inheriting a large instructions file was enough to
                // end its sweeps before they read any code.
                isolate::strip_agent_instructions(worktree.path());
                Some(worktree)
            }
            Err(error) => {
                return not_swept(format!(
                    "{} cannot be run read-only, so its sweep needs a throwaway git worktree, \
                     which could not be created: {error}",
                    vendor.label()
                ));
            }
        }
    } else {
        None
    };

    // Anchors are verified against whatever the model actually read.
    let reviewed = isolation
        .as_ref()
        .map_or(request.repo, |worktree| worktree.path());

    let outcome = match vendor {
        Vendor::Claude => claude::sweep(ClaudeSweep {
            repo: reviewed,
            lane: request.lane,
            model,
            effort: request.effort,
            brief: &brief,
            timeout: request.timeout,
            max_turns: request.max_turns,
            binary: None,
            api_key: request.api_key,
        })
        .await
        .map(|r| (r.findings.findings, r.turns, r.salvaged)),
        Vendor::Codex => codex::sweep(CodexSweep {
            repo: reviewed,
            model,
            effort: request.effort,
            brief: &brief,
            timeout: request.timeout,
            binary: None,
        })
        .await
        .map(|r| (r.findings.findings, None, false)),
        Vendor::Kilo => kilo::sweep(KiloSweep {
            worktree: reviewed,
            model,
            effort: request.effort,
            brief: &brief,
            timeout: request.timeout,
            binary: None,
        })
        .await
        .map(|r| (r.findings.findings, None, false)),
    };

    let (raw, turns, salvaged) = match outcome {
        Ok(outcome) => outcome,
        Err(error) => return not_swept(error.to_string()),
    };

    let (findings, rejected) = verify_all(reviewed, request.lane, &ModelId::new(&model_label), raw);

    LaneReport {
        lane: request.lane.title().to_string(),
        model: model_label,
        commit,
        scope: request.scope.map(str::to_string),
        status: Status::Swept { turns, salvaged },
        findings,
        rejected,
    }
}

/// The commit HEAD points at, or nothing for a directory that is not a git
/// repository. Never an error: provenance is worth recording, not worth
/// failing a sweep over.
fn reviewed_commit(repo: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!hash.is_empty()).then_some(hash)
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
/// Probe every provider CLI, returning what each said.
///
/// Data rather than printed text, so the desktop app and the command line can
/// share one implementation instead of drifting apart on what "available" means.
///
/// Note what a success here does **not** prove: `--version` works fine for a CLI
/// that is installed but not signed in. Only a real sweep proves authentication.
pub async fn probe_all() -> Vec<(&'static str, Result<String, String>)> {
    let (claude, codex, kilo) = tokio::join!(claude::probe(), codex::probe(), kilo::probe());
    vec![
        ("claude", claude.map_err(|e| e.to_string())),
        ("codex", codex.map_err(|e| e.to_string())),
        ("kilo", kilo.map_err(|e| e.to_string())),
    ]
}

/// Confirm the provider CLIs can be started before a run commits to them.
pub async fn preflight() -> Result<()> {
    let probes = probe_all().await;
    let total = probes.len();
    let mut usable = 0;
    for (name, probe) in probes {
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
{usable} of {total} provider CLIs can be started."
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
    fn kilo_is_selected_by_prefix_and_needs_isolation_because_it_has_no_read_only_mode() {
        assert_eq!(
            Vendor::parse("kilo:anthropic/claude-sonnet-4-5"),
            (Vendor::Kilo, "anthropic/claude-sonnet-4-5")
        );
        assert!(Vendor::Kilo.needs_isolation());
        assert!(!Vendor::Claude.needs_isolation());
        assert!(!Vendor::Codex.needs_isolation());
    }

    #[test]
    fn only_kilo_needs_the_schema_spelled_out_in_its_prompt() {
        assert!(!Vendor::Kilo.enforces_schema());
        assert!(Vendor::Claude.enforces_schema());
        assert!(Vendor::Codex.enforces_schema());
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
