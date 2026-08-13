//! One lane sweep, end to end: brief the model, run it, verify what comes back.

mod agents;
mod isolate;
mod precheck;
mod revision;
mod vendor;
mod verify;

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use bugsleuth_domain::{Lane, ModelId, RawFinding};
use bugsleuth_provider::claude::{self, ClaudeSweep};
use bugsleuth_provider::codex::{self, CodexSweep};
use bugsleuth_provider::cursor::{self, CursorSweep};
use bugsleuth_provider::kilo::{self, KiloSweep};
use bugsleuth_provider::kimi::{self, KimiSweep};
use bugsleuth_provider::process::redact_secrets;

use crate::brief;
use crate::report::{LaneReport, Status};
// `clean_revision` is re-exported so `orchestrate::persist` reaches it as
// `crate::sweep::clean_revision`, unchanged by the split.
pub use agents::cannot_delegate;
pub(crate) use agents::support as agent_support;
pub use precheck::selected as precheck_selected;
pub(crate) use revision::clean_revision;
use revision::reviewed_commit;
pub use vendor::{Vendor, resolved_label};

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
    /// Explicit provider CLI path for tests; real runs use discovery.
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
    use_agents: bool,
) -> Result<(Vec<RawFinding>, Option<u32>, bool, Option<String>), bugsleuth_provider::ProviderError>
{
    match vendor {
        Vendor::Claude => claude::sweep(ClaudeSweep {
            repo: reviewed,
            lane: request.lane,
            model,
            effort: request.effort,
            use_agents,
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
        .map(|r| (r.findings.findings, None, r.salvaged, None)),
        Vendor::Kilo => kilo::sweep(KiloSweep {
            worktree: reviewed,
            model,
            effort: request.effort,
            brief,
            timeout: request.timeout,
            binary: request.binary,
        })
        .await
        .map(|r| (r.findings.findings, None, r.salvaged, None)),
        // No effort: Kimi has no reasoning-depth flag, and inventing one would
        // send a value its CLI rejects. `plan::check_effort` refuses the
        // combination before a sweep is paid for.
        Vendor::Kimi => kimi::sweep(KimiSweep {
            worktree: reviewed,
            model,
            brief,
            timeout: request.timeout,
            binary: request.binary,
        })
        .await
        .map(|r| (r.findings.findings, None, false, None)),
        Vendor::Cursor => cursor::sweep(CursorSweep {
            worktree: reviewed,
            model,
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
    run_with_agents(request, false).await
}

fn excluded_scope<'a>(scope: &str, excluded_paths: &'a [String]) -> Option<&'a str> {
    let scope = scope
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_lowercase();
    excluded_paths.iter().find_map(|excluded| {
        let normalized = excluded.trim_matches('/').to_lowercase();
        (scope == normalized || scope.starts_with(&format!("{normalized}/")))
            .then_some(excluded.as_str())
    })
}

pub(crate) async fn run_with_agents(request: Request<'_>, use_agents: bool) -> LaneReport {
    // Both halves are used: the vendor picks the adapter, and the model is what
    // that adapter passes to its CLI. Taking only the vendor and handing the
    // whole spec on is the 0.2.19 regression — see [`invoke_vendor`].
    let (vendor, model) = Vendor::parse(request.model);
    let model_label = resolved_label(request.model);
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
        excluded_paths: Vec::new(),
        status: Status::NotSwept { reason },
        findings: vec![],
        rejected: vec![],
        usage: None,
    };

    let agents_instruction = if use_agents {
        // The reason comes from the same answer as the capability, so a vendor
        // that cannot delegate is refused in its own terms rather than in
        // Kilo's — "kimi's read-only Ask agent" described a thing Kimi has no
        // concept of. `plan` refuses this before any quota is spent; reaching
        // here means a configuration that bypassed it.
        match agents::support(vendor, model) {
            Ok(instruction) => Some(instruction),
            Err(reason) => return not_swept(reason.to_string()),
        }
    } else {
        None
    };
    let brief = brief::build_with_agents(
        request.lane,
        request.scope,
        vendor.enforces_schema(),
        agents_instruction,
    );

    // Kilo's sweep runs under the globally configured `ask` agent, whose
    // `deny` rules the CLI honours even under `--auto` — measured against the
    // real binary with the flags in `kilo::BASE_FLAGS`, not inferred from the
    // help text, which describes `--auto` as approving everything. What it
    // actually overrides is `ask`, never `deny`. Under those flags an edit was
    // refused, `bash` was refused, and a read outside `--dir` was refused by
    // the `external_directory` rule.
    //
    // The gate is the user's own config, so verify it rather than trust it. A
    // reviewed repository is attacker input: shell, external paths, edits,
    // skills, subagents, network and unknown future tools all need to default
    // to denied, rather than inheriting one developer's current setup.
    if vendor == Vendor::Kilo
        && let Some(error) = precheck::kilo_permission_error()
    {
        return not_swept(error);
    }

    // A vendor that cannot be run read-only gets a throwaway checkout instead.
    // The worktree is held for the whole sweep and deletes itself on drop.
    let isolation = match isolate::checkout_for(vendor, request.repo) {
        Ok(isolation) => isolation,
        Err(reason) => return not_swept(reason),
    };
    if let (Some(scope), Some(isolated)) = (request.scope, isolation.as_ref())
        && let Some(excluded) = excluded_scope(scope, &isolated.excluded_paths)
    {
        return not_swept(format!(
            "requested scope {scope} was not reviewed because provider isolation removed {excluded}"
        ));
    }

    // Anchors are verified against whatever the model actually read.
    let reviewed = isolation
        .as_ref()
        .map_or(request.repo, |isolated| isolated.worktree.path());

    let outcome = invoke_vendor(vendor, model, reviewed, &request, &brief, use_agents).await;
    // A transient blip — a silent overloaded response, an empty completion, a
    // rate limit — is worth one more attempt before a whole lane reads as never
    // run. The decision is the provider's own: `is_transient` knows which of its
    // failures look the same every time and which do not. Only one retry: a
    // second identical failure means the condition is not momentary, and paying
    // for a third is what `--resume` is for.
    let outcome = match outcome {
        Ok(outcome) => Ok(outcome),
        Err(error) if error.is_transient() => {
            invoke_vendor(vendor, model, reviewed, &request, &brief, use_agents).await
        }
        Err(error) => Err(error),
    };

    let (raw, turns, salvaged, usage) = match outcome {
        Ok(outcome) => outcome,
        // A CLI can leak its OAuth tokens in an error (Kilo did, on a
        // credential-store failure), and this is the last boundary before that
        // text is stored and shown — so scrub anything token-shaped from it.
        Err(error) => return not_swept(redact_secrets(&error.to_string())),
    };

    let (findings, rejected) =
        verify::verify_all(reviewed, request.lane, &ModelId::new(&model_label), raw);

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
        excluded_paths: isolation
            .as_ref()
            .map_or_else(Vec::new, |isolated| isolated.excluded_paths.clone()),
        status: Status::Swept { turns, salvaged },
        findings,
        rejected,
        usage,
    }
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
/// that is installed but not signed in. The desktop's selected-provider check
/// proves that.
pub async fn probe_all() -> Vec<(&'static str, Result<String, String>)> {
    let (claude, codex, kilo, kimi, cursor) = tokio::join!(
        claude::probe(),
        codex::probe(),
        kilo::probe(),
        kimi::probe(),
        cursor::probe()
    );
    vec![
        ("claude", claude.map_err(|e| e.to_string())),
        ("codex", codex.map_err(|e| e.to_string())),
        ("kilo", kilo.map_err(|e| e.to_string())),
        ("kimi", kimi.map_err(|e| e.to_string())),
        ("cursor", cursor.map_err(|e| e.to_string())),
    ]
}

/// Prove each vendor is signed in, by asking it something.
///
/// The counterpart to [`probe_all`], and the honest one. Probing answers "can
/// this CLI start", which every one of them can while signed out; this asks for
/// one word back, which only a real session can produce. Costs a trivial model
/// call per vendor. A desktop run performs it for its selected providers; the
/// button performs it for every one.
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
    println!("This does not prove they are signed in; use Check sign-in for that.");
    if usable == 0 {
        std::process::exit(2);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "sweep/vendor_tests.rs"]
mod vendor_tests;

#[cfg(test)]
mod isolation_tests;
