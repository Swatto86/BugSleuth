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

/// The `vendor:model` a spec resolves to, exactly as a report records it.
///
/// One function, because the label was being built in one place and compared
/// against a raw config string in another. A unit configured as `sonnet`
/// produced a report saying `claude:sonnet`, and the equality test between them
/// was never true — so a cancelled run counted every finished sweep as still
/// outstanding and told the reader that lanes it had already swept were not
/// reached.
#[must_use]
pub fn resolved_label(spec: &str) -> String {
    let (vendor, model) = Vendor::parse(spec);
    format!("{}:{model}", vendor.label())
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
    /// Explicit path to the vendor CLI, overriding discovery. `None` in every
    /// real run; the adapters have always taken one, and without a way to set it
    /// nothing could check what argv a sweep actually builds.
    pub binary: Option<&'a str>,
}

/// Run the vendor sweep against the already-isolated directory.
///
/// `model` is the spec with its vendor prefix removed, and is a parameter rather
/// than read back off `request` for the reason it shipped broken in 0.2.19:
/// `request.model` is BugSleuth's own `vendor:model` notation, which no CLI
/// knows. Every Kilo and Codex sweep of that release was refused with
/// `Model not found: kilo:kilo/...` before it read a line of code.
async fn invoke_vendor(
    vendor: Vendor,
    model: &str,
    reviewed: &Path,
    request: &Request<'_>,
    brief: &str,
) -> Result<(Vec<RawFinding>, Option<u32>, bool, Option<String>), bugsleuth_provider::ProviderError>
{
    match vendor {
        Vendor::Claude => claude::sweep(ClaudeSweep {
            repo: reviewed,
            lane: request.lane,
            model,
            effort: request.effort,
            brief,
            timeout: request.timeout,
            max_turns: request.max_turns,
            binary: request.binary,
            api_key: request.api_key,
        })
        .await
        .map(|r| {
            (
                r.findings.findings,
                r.turns,
                r.salvaged,
                r.usage.map(|u| u.to_text()),
            )
        }),
        Vendor::Codex => codex::sweep(CodexSweep {
            repo: reviewed,
            model,
            effort: request.effort,
            brief,
            timeout: request.timeout,
            binary: request.binary,
        })
        .await
        .map(|r| (r.findings.findings, None, false, None)),
        Vendor::Kilo => kilo::sweep(KiloSweep {
            worktree: reviewed,
            model,
            effort: request.effort,
            brief,
            timeout: request.timeout,
            binary: request.binary,
        })
        .await
        .map(|r| (r.findings.findings, None, false, None)),
    }
}

/// Run the sweep. Never returns an error for a failed sweep — a failure is a
/// *reported state*, because the one outcome this tool must never produce is a
/// lane that quietly looks clean when it never ran.
pub async fn run(request: Request<'_>) -> LaneReport {
    // Both halves are used: the vendor picks the adapter, and the model is what
    // that adapter passes to its CLI. Taking only the vendor and handing the
    // whole spec on is the 0.2.19 regression — see [`invoke_vendor`].
    let (vendor, model) = Vendor::parse(request.model);
    let model_label = resolved_label(request.model);
    let brief = brief::build(request.lane, request.scope, vendor.enforces_schema());
    // Recorded before anything runs, so even a failed sweep says what tree it
    // was pointed at.
    let commit = reviewed_commit(request.repo);
    // The revision this sweep may later be reused for. Captured before anything
    // runs and confirmed unchanged after, so a sweep that read a tree edited
    // under it is never handed back as a review of the edited tree.
    let cache_revision_before = clean_revision(request.repo);

    let not_swept = |reason: String| LaneReport {
        lane: request.lane.title().to_string(),
        model: model_label.clone(),
        commit: commit.clone(),
        cache_revision: None,
        scope: request.scope.map(str::to_string),
        status: Status::NotSwept { reason },
        findings: vec![],
        rejected: vec![],
        usage: None,
    };

    // Kilo's sweep runs under the globally configured `ask` agent, whose
    // `deny` rules the CLI honours even under `--auto` — measured against the
    // real binary with the flags in `kilo::BASE_FLAGS`, not inferred from the
    // help text, which describes `--auto` as approving everything. What it
    // actually overrides is `ask`, never `deny`. Under those flags an edit was
    // refused, `bash` was refused, and a read outside `--dir` was refused by
    // the `external_directory` rule.
    //
    // `webfetch` was **not** refused, so a Kilo sweep can still reach the
    // network while holding the reviewed repository's text — which is attacker
    // input. That is the one restriction Kilo cannot be made to enforce here,
    // so it is the one this check still refuses on, rather than the blanket
    // refusal that predated the measurement.
    //
    // The gate is the user's own config, so verify it rather than trust it: a
    // machine whose `ask` agent permits the network must not sweep silently.
    if vendor == Vendor::Kilo
        && let Some(gap) = kilo::preflight::network_gap()
    {
        return not_swept(format!(
            "Kilo sweeps need the network denied, because a sweep holds the reviewed \
             repository's text and could send it out: {gap}. Add \"webfetch\": \"deny\" and \
             \"websearch\": \"deny\" to that agent's permission block."
        ));
    }

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

    let outcome = invoke_vendor(vendor, model, reviewed, &request, &brief).await;

    let (raw, turns, salvaged, usage) = match outcome {
        Ok(outcome) => outcome,
        Err(error) => return not_swept(error.to_string()),
    };

    let (findings, rejected) = verify_all(reviewed, request.lane, &ModelId::new(&model_label), raw);

    // Only reusable if the repository was clean at the start and is still at the
    // same clean revision now: a HEAD that moved, or a working tree that was
    // edited, means this sweep no longer describes the current tree.
    let cache_revision = cache_revision_before
        .filter(|before| clean_revision(request.repo).as_ref() == Some(before));

    LaneReport {
        lane: request.lane.title().to_string(),
        model: model_label,
        commit,
        cache_revision,
        scope: request.scope.map(str::to_string),
        status: Status::Swept { turns, salvaged },
        findings,
        rejected,
        usage,
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

/// The commit HEAD points at, but only when the working tree is clean.
///
/// A sweep is reusable only if it can be pinned to an exact source revision.
/// A dirty tree cannot be — its content is not any commit — so this returns
/// `None` for a dirty or non-git directory, and only a clean checkout yields a
/// revision a later run can compare against.
pub(crate) fn clean_revision(repo: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() || !output.stdout.is_empty() {
        return None;
    }
    reviewed_commit(repo)
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

/// Prove each vendor is signed in, by asking it something.
///
/// The counterpart to [`probe_all`], and the honest one. Probing answers "can
/// this CLI start", which every one of them can while signed out; this asks for
/// one word back, which only a real session can produce. Costs a trivial model
/// call per vendor, so it is offered rather than run automatically.
pub async fn check_signin() -> Vec<(&'static str, bugsleuth_provider::signin::SignIn)> {
    bugsleuth_provider::signin::check_all().await
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
mod tests;
