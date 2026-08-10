//! Applying the fix prompt to the repository it came from.
//!
//! Everything else in BugSleuth is careful never to modify the code it reads: a
//! read-only sweep cannot write, and a vendor that cannot be sandboxed reviews a
//! throwaway worktree. This is the one deliberate exception, and it exists
//! because the alternative — copy the prompt, open another tool, paste it — is
//! what everyone was doing anyway.
//!
//! The safety story is git, not a sandbox. A model given write access to a real
//! checkout can do anything to it, so the only claim worth making is that
//! whatever it does is *visible and reversible*:
//!
//! - The working tree must be clean before anything starts. Uncommitted work of
//!   your own would otherwise be mixed into the model's changes with no way to
//!   tell them apart, and no way to revert one without the other.
//! - What changed is reported from git afterwards, never from the model's own
//!   account — including changes it committed, which `git status` alone would
//!   show as a clean tree and therefore as nothing having happened.

use std::path::Path;
use std::time::Duration;

use anyhow::Context;

use crate::sweep::Vendor;

mod attribution;
mod network;
mod observed;
mod push;
mod remote;
mod tag;
use attribution::{attributed_since, strip_attribution};
use observed::{changed_since, commits_since, dirty_files, git, summarise, theirs};
pub use push::PushOutcome;
pub use tag::TagOutcome;

/// Where the repository stood before the model touched it.
///
/// A plain `Option<String>` conflated two very different states: "there is no
/// initial commit yet" and "rev-parse HEAD failed for some other reason". The
/// first is a real starting point — the empty tree — that the model can commit
/// against, and treating it as "no baseline" made a freshly committed initial
/// commit read as no change at all.
#[derive(Debug, Clone)]
pub(crate) enum Baseline {
    /// HEAD resolved to this commit before the apply.
    Commit(String),
    /// The repository has a `.git` but no commit yet — an unborn branch. What
    /// the model commits is measured against Git's empty tree.
    Unborn,
}

pub struct ApplyRequest<'a> {
    /// The repository to edit. Its working tree must be clean.
    pub repo: &'a Path,
    /// A `vendor:model` spec, exactly as the model matrix writes them.
    pub model: &'a str,
    /// Reasoning effort. Empty means the vendor's own default.
    pub effort: &'a str,
    /// The handoff prompt, as written by [`crate::handoff`].
    pub prompt: &'a str,
    pub timeout: Duration,
    /// Turn ceiling, for the vendor that has one.
    pub max_turns: u32,
    /// Stops the provider and any in-flight remote publication.
    pub cancel: crate::cancel::Cancel,
    /// Push what the model committed to the branch's existing upstream.
    ///
    /// Off unless the user turned it on. Everything else an apply does is
    /// undoable with `git reset`; this is the one step that is not, so it is
    /// asked for explicitly and refused in every case the answer is not
    /// obvious — see [`push`].
    pub push: bool,
    /// After a successful push, tag the published commits so the repository's
    /// own CI cuts a release from them.
    ///
    /// Only ever *after* a push, never instead of one: a tag whose commits are
    /// not on the remote starts a build of a ref the runner cannot fetch. See
    /// [`tag`].
    pub tag: bool,
}

pub struct ApplyReport {
    /// The model's own account of what it did. Not evidence — read it beside
    /// `changed_files`, which comes from git.
    pub text: String,
    /// Every file that differs from where the repository started, whether the
    /// change was committed or not.
    pub changed_files: Vec<String>,
    /// Commits made during the run. The prompt asks for one per defect, so a
    /// clean tree afterwards is the expected outcome rather than a suspicious one.
    pub commits: usize,
    /// Subjects of commits whose authorship trailer was removed after the fact.
    ///
    /// This tool does not attribute an AI in someone else's repository. The CLI
    /// setting removes the cause for the vendor that has one; this catches the
    /// rest, including a model that types the trailer out by hand. Seven such
    /// commits reached a real repository before any of this existed, and were
    /// caught in the minute before they were pushed.
    ///
    /// Named rather than silently fixed: a tool that rewrites your commits and
    /// says nothing is worse than one that leaves them alone.
    pub stripped: Vec<String>,
    /// Commits still carrying a trailer, because they could not be rewritten —
    /// they had already reached a remote, or HEAD was detached. Reported so the
    /// user can act, since the tool must not fork published history to fix its
    /// own mess.
    pub attributed: Vec<String>,
    /// What became of the request to publish these commits.
    ///
    /// A refusal here is not a failed apply — the fixes are committed either
    /// way — so it is carried in the report rather than raised as an error.
    pub push: PushOutcome,
    /// What became of the request to tag the published commits for release.
    pub tag: TagOutcome,
}

/// Run the fixes, then report what actually changed.
///
/// # Errors
/// Refuses before spending anything if the repository is not a git checkout or
/// its working tree is dirty, and reports a vendor failure as an error rather
/// than as an empty result.
pub async fn apply(request: ApplyRequest<'_>) -> anyhow::Result<ApplyReport> {
    let repo = request.repo;
    if !repo.join(".git").exists() {
        anyhow::bail!(
            "{} is not a git repository. Applying fixes edits your files in place, and git is \
             the only thing that makes that reversible — so it is refused without one.",
            repo.display()
        );
    }

    let dirty = theirs(dirty_files(&git(repo, &["status", "--porcelain", "-z"])?));
    if !dirty.is_empty() {
        anyhow::bail!(
            "the working tree has uncommitted changes, so applying fixes is refused: your work \
             and the model's would be mixed together with no way to revert one without the \
             other. Commit or stash these first — {}",
            summarise(&dirty)
        );
    }

    // Where the repository started. Compared against afterwards, because the
    // prompt asks the model to commit each fix and a committed change leaves
    // `git status` clean — reporting from status alone would say nothing
    // happened after a model had rewritten half the tree.
    let base = baseline(repo)?;

    let (vendor, model) = Vendor::parse(request.model);
    let provider = run_provider(&request, vendor, model);
    tokio::pin!(provider);
    let attempt = tokio::select! {
        biased;
        () = request.cancel.cancelled() => anyhow::bail!(cancelled_message()),
        attempt = &mut provider => attempt,
    };

    // A failure is not "nothing happened". The invocation is killed on timeout
    // and can fail after the model has already rewritten half the tree, and an
    // error on its own would send someone away believing their repository was
    // untouched. Whatever git can see is named in the error too.
    let text = match attempt {
        Ok(text) => text,
        // A failure is not "nothing happened" — name whatever git can still see.
        // If git itself cannot be read afterwards, say the state is unknown
        // rather than claim nothing changed.
        Err(error) => match changed_since(repo, &base) {
            Ok(changed) => anyhow::bail!(failure_message(&error.to_string(), &changed)),
            Err(_) => anyhow::bail!(failure_message_unknown(&error.to_string())),
        },
    };

    // Attribution comes off before anything else is reported, so what the user
    // reads describes the repository as it now stands. Only commits this apply
    // created, only while they are still local — see `strip_attribution`.
    //
    // A failure here is not a failed apply: the fixes are real and already
    // committed. What is left is named instead, which is the whole difference
    // between a tool that quietly forks published history and one that says
    // "these two are yours to deal with". An unborn baseline is included: its
    // initial commit must not escape both the stripping and the report.
    if request.cancel.stopped() {
        anyhow::bail!(cancelled_message());
    }
    let (stripped, attributed) =
        match strip_attribution(repo, &base, &request.cancel, request.timeout).await {
            Ok(stripped) => (stripped, vec![]),
            // Stripping was refused (published history, or a detached HEAD) — report
            // what it could not reach. But if the rescan itself cannot read history,
            // that is not "nothing to report": it means the check never ran, so fail
            // closed rather than push and tag on an unverified repository.
            Err(_) => {
                let attributed = attributed_since(repo, &base).with_context(|| {
                "the model's fixes are committed to your repository, but BugSleuth could not read \
                 its history to check them for AI attribution — so nothing was pushed or tagged. \
                 Inspect it with `git log` before publishing."
            })?;
                (vec![], attributed)
            }
        };

    // Read what git actually shows, once, after the provider and the strip. A
    // failure here is not an empty repository — it means the repository could
    // not be inspected at all, so the report must not claim "nothing changed"
    // and nothing may be published. The same values feed the report below.
    if request.cancel.stopped() {
        anyhow::bail!(cancelled_message());
    }
    let changed_files = changed_since(repo, &base).with_context(|| {
        "the model finished, but BugSleuth could not determine what changed in your repository; \
         inspect it manually — nothing was pushed or tagged"
    })?;
    let commits = commits_since(repo, &base).with_context(|| {
        "the model finished, but BugSleuth could not count the commits it made; inspect the \
         repository manually — nothing was pushed or tagged"
    })?;

    // After stripping, deliberately: the trailers come off before anything is
    // published, and `attributed` is what stripping could not reach — which is
    // itself a reason to refuse the push rather than to publish and mention it.
    let pushed = if request.push && request.cancel.stopped() {
        PushOutcome::Refused("publication was cancelled before the push started".to_string())
    } else if request.push {
        push::push(
            repo,
            &base,
            commits,
            &attributed,
            &request.cancel,
            request.timeout,
        )
        .await
    } else {
        PushOutcome::NotRequested
    };

    let cancel = &request.cancel;
    let timeout = request.timeout;
    let tagged = match to_tag(request.tag, &pushed) {
        Some(_) if request.cancel.stopped() => {
            TagOutcome::Refused("publication was cancelled before tagging started".to_string())
        }
        Some((remote, oid)) => tag::tag(repo, true, remote, oid, cancel, timeout).await,
        None if request.tag => tag::tag(repo, false, "", "", cancel, timeout).await,
        None => TagOutcome::NotRequested,
    };

    Ok(ApplyReport {
        text,
        changed_files,
        commits,
        stripped,
        attributed,
        push: pushed,
        tag: tagged,
    })
}

async fn run_provider(
    request: &ApplyRequest<'_>,
    vendor: Vendor,
    model: &str,
) -> Result<String, bugsleuth_provider::ProviderError> {
    match vendor {
        Vendor::Claude => {
            bugsleuth_provider::claude::apply(bugsleuth_provider::claude::ApplyRequest {
                repo: request.repo,
                model,
                effort: request.effort,
                prompt: request.prompt,
                timeout: request.timeout,
                max_turns: request.max_turns,
            })
            .await
        }
        Vendor::Codex => {
            bugsleuth_provider::codex::apply(
                request.repo,
                model,
                request.effort,
                request.prompt,
                request.timeout,
            )
            .await
        }
        Vendor::Kilo => {
            bugsleuth_provider::kilo::apply(
                request.repo,
                model,
                request.effort,
                request.prompt,
                request.timeout,
            )
            .await
        }
    }
}

fn cancelled_message() -> &'static str {
    "the apply was stopped. The model was killed part-way through editing the repository — check `git status` and `git log` to see what it had already changed."
}

/// The upstream a tag may be published to, or `None` when this must not be
/// tagged at all.
///
/// The ordering rule, on its own so it can be tested without a model, a
/// repository or a remote — everything `apply` would otherwise need before this
/// single decision could be exercised.
///
/// A tag is the trigger for someone else's release pipeline. Pushed ahead of its
/// commits it starts a build of a ref the runner cannot fetch, so it is only
/// ever reached through a push that actually succeeded — and the remote plus
/// exact published commit come from that push rather than being worked out a
/// second time. Every other push
/// outcome (a detached HEAD, no upstream, a rejected push, a commit still
/// crediting a tool) is a reason not to release, not merely a reason not to push.
fn to_tag(requested: bool, pushed: &PushOutcome) -> Option<(&str, &str)> {
    match (requested, pushed) {
        // The exact remote and confirmed object from the push. Neither is
        // re-derived from mutable repository state before release selection.
        (true, PushOutcome::Pushed { remote, oid, .. }) => Some((remote, oid)),
        _ => None,
    }
}

/// Read where the repository stands before the apply.
///
/// `git rev-parse --verify HEAD` resolves to the current commit or fails when
/// the branch is unborn; `--quiet` keeps it from printing an error. A failure is
/// not automatically "unborn", though — git could be broken — so it is
/// confirmed with `rev-list --all --count`, which is `0` only when the
/// repository genuinely has no commits. Anything else is propagated rather than
/// silently treated as an empty history.
fn baseline(repo: &Path) -> anyhow::Result<Baseline> {
    if let Ok(out) = git(repo, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
        let id = out.trim().to_string();
        if !id.is_empty() {
            return Ok(Baseline::Commit(id));
        }
    }
    let count = git(repo, &["rev-list", "--all", "--count"])?;
    if count.trim() == "0" {
        Ok(Baseline::Unborn)
    } else {
        anyhow::bail!(
            "could not resolve HEAD in {} even though it has commits — is HEAD detached or the \
             repository corrupt?",
            repo.display()
        );
    }
}

/// A vendor failure, plus whatever it had already done to the repository.
///
/// A failure is not "nothing happened". The invocation is killed on timeout and
/// can fail after the model has rewritten half the tree, so an error on its own
/// would send someone away believing their checkout was untouched — the same
/// "absence of a result read as a result" this whole project exists to stop.
fn failure_message(error: &str, changed: &[String]) -> String {
    let error = bugsleuth_provider::process::redact_secrets(error);
    match changed.len() {
        0 => format!("{error} — git shows no files changed."),
        n => format!(
            "{error} — but {n} file{} had already changed when it stopped: {}. \
             Check `git status` and `git diff` before running anything again.",
            if n == 1 { "" } else { "s" },
            summarise(changed)
        ),
    }
}

/// The failure message when git could not be read afterwards either.
///
/// The vendor failed *and* the repository could not be inspected, so there is no
/// honest "what changed" list to give. It must not fall back to "no files
/// changed" — that would claim the tree is clean when the truth is it is
/// unknown, the exact false-clean this project exists to stop.
fn failure_message_unknown(error: &str) -> String {
    let error = bugsleuth_provider::process::redact_secrets(error);
    format!(
        "{error} — and BugSleuth could not read the repository afterwards to see what changed. \
         Check `git status` and `git diff` before running anything again."
    )
}

#[cfg(test)]
#[path = "apply/tests.rs"]
mod tests;
