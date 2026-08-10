//! Publishing what an apply committed — only when asked, and only when it is
//! safe to do so.
//!
//! Everything else about applying fixes is reversible: the changes are in git,
//! on your machine, and `git reset` undoes them. Pushing is the one step that
//! is not. Once commits reach a remote they can be fetched by anyone watching
//! it, and a later rewrite does not recall them.
//!
//! So this is deliberately timid. It pushes the current branch to the upstream
//! it already has, and nothing else:
//!
//! - never `--force`, so a rejected push stays rejected and is reported;
//! - never `-u`, and never a guessed remote — a branch with no upstream is
//!   left alone rather than published somewhere chosen on its behalf;
//! - never from a detached HEAD, where there is no branch to publish;
//! - never when a commit still credits a tool for the work, because that is
//!   exactly the thing that cannot be taken back once it is public.

use std::path::Path;
use std::time::Duration;

use crate::cancel::Cancel;

use super::{Baseline, network, observed::git};

/// What became of the request to push. Every variant is a normal outcome of a
/// successful apply — the fixes are already committed either way, so a refusal
/// here is reported rather than raised as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// The setting is off, so nothing was attempted.
    NotRequested,
    /// Asked for, but the apply committed nothing to publish.
    NothingToPush,
    /// Asked for and declined, for the reason given.
    Refused(String),
    Pushed {
        branch: String,
        upstream: String,
        /// The exact remote the push used. Carried so a later tag goes to the
        /// same place, rather than being re-derived from `upstream` by splitting
        /// on '/', which truncates a remote name that itself contains a slash.
        remote: String,
        /// The frozen apply tip confirmed on the remote. A later release must
        /// target this object rather than re-reading mutable `HEAD`.
        oid: String,
    },
    /// git ran and the remote is confirmed still unchanged: a genuine rejection.
    /// Left for the user: the fix is a fetch and a rebase, or a force-push, and
    /// neither is this tool's to choose.
    Failed(String),
    /// The push errored and the remote could not be confirmed either way. The
    /// commits may or may not be published, so this must not be retried
    /// automatically and must never be described as a definite failure.
    Unknown {
        branch: String,
        upstream: String,
        error: String,
    },
}

/// Why this must not be pushed, or `None` when the observed apply is publishable.
///
/// Split from the git calls below so the guards can be tested without a remote.
fn blocked(commits: usize, attributed: &[String]) -> Option<PushOutcome> {
    if commits == 0 {
        return Some(PushOutcome::NothingToPush);
    }
    // The one refusal that is not about git mechanics. A trailer that could not
    // be stripped is a trailer on a commit that had already been published or
    // could not be rewritten — and pushing it is what makes it permanent in a
    // history whose owner never agreed to credit a tool.
    if !attributed.is_empty() {
        return Some(PushOutcome::Refused(format!(
            "{} still {} a tool for the work and could not be rewritten. Nothing was pushed: \
             publishing is the step that cannot be undone. Strip the trailers, then push by hand.",
            match attributed.len() {
                1 => "1 commit".to_string(),
                n => format!("{n} commits"),
            },
            if attributed.len() == 1 {
                "credits"
            } else {
                "credit"
            },
        )));
    }
    None
}

/// The remote and remote-ref an upstream branch tracks, or a refusal to push.
fn upstream_remote_ref(
    repo: &Path,
    branch: &str,
    upstream: &str,
) -> Result<(String, String), PushOutcome> {
    let location = format!("refs/heads/{branch}");
    git(
        repo,
        &[
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream:remoteref)",
            &location,
        ],
    )
    .and_then(|value| {
        let Some((remote, reference)) = value.trim().split_once('\0') else {
            anyhow::bail!("git did not report the upstream remote and ref");
        };
        if remote.is_empty() || reference.is_empty() {
            anyhow::bail!("git reported an incomplete upstream remote or ref");
        }
        Ok((remote.to_string(), reference.to_string()))
    })
    .map_err(|error| {
        PushOutcome::Refused(format!(
            "could not establish the live location of {upstream}: {error}. Nothing was pushed."
        ))
    })
}

/// The single object ID the upstream ref currently points at on the remote, or
/// a refusal when it cannot be read unambiguously.
async fn upstream_live_tip(
    repo: &Path,
    remote: &str,
    reference: &str,
    upstream: &str,
    cancel: &Cancel,
    timeout: Duration,
) -> Result<String, PushOutcome> {
    network::git(repo, &["ls-remote", remote, reference], cancel, timeout)
        .await
        .and_then(|value| {
            let mut ids = value
                .lines()
                .filter_map(|line| line.split_whitespace().next());
            let Some(id) = ids.next().filter(|id| !id.is_empty()) else {
                return Err(network::Error::Failed(
                    "the upstream ref does not exist".to_string(),
                ));
            };
            if ids.next().is_some() {
                return Err(network::Error::Failed(
                    "git reported more than one upstream object ID".to_string(),
                ));
            }
            Ok(id.to_string())
        })
        .map_err(|error| {
            PushOutcome::Refused(format!(
                "could not establish the live tip of {upstream}: {error}. Nothing was pushed."
            ))
        })
}

fn head_oid(repo: &Path) -> Option<String> {
    let id = git(repo, &["rev-parse", "HEAD"]).ok()?;
    let id = id.trim();
    (!id.is_empty()).then_some(id.to_string())
}

/// Push the branch the apply committed on, if it may be pushed.
pub(super) async fn push(
    repo: &Path,
    base: &Baseline,
    commits: usize,
    attributed: &[String],
    cancel: &Cancel,
    timeout: Duration,
) -> PushOutcome {
    if let Some(stop) = blocked(commits, attributed) {
        return stop;
    }

    // `symbolic-ref` rather than `rev-parse --abbrev-ref`, which answers the
    // literal string "HEAD" on a detached checkout — a name that would then be
    // handed to `git push` as though it were a branch.
    let branch = match git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Ok(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => {
            return PushOutcome::Refused(
                "HEAD is detached, so there is no branch to push. The commits are safe in the \
                 repository — check them out onto a branch first."
                    .to_string(),
            );
        }
    };

    let upstream = match git(
        repo,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    ) {
        Ok(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => {
            return PushOutcome::Refused(format!(
                "{branch} has no upstream branch, so there is nowhere it is already agreed to \
                 publish to. Nothing was pushed — run `git push -u <remote> {branch}` once, and \
                 this will follow it after that."
            ));
        }
    };

    let (remote, reference) = match upstream_remote_ref(repo, &branch, &upstream) {
        Ok(location) => location,
        Err(refusal) => return refusal,
    };
    let live_upstream_tip =
        match upstream_live_tip(repo, &remote, &reference, &upstream, cancel, timeout).await {
            Ok(tip) => tip,
            Err(refusal) => return refusal,
        };

    let Baseline::Commit(base) = base else {
        return PushOutcome::Refused(
            "the branch had no starting commit, so publication cannot be limited to this apply"
                .to_string(),
        );
    };
    if live_upstream_tip != *base {
        return PushOutcome::Refused(format!(
            "the branch was not synchronized with {upstream} when this apply began. Nothing was pushed, because doing so would also publish commits that predate this apply."
        ));
    }

    // Freeze the object before starting the network operation. The shared
    // explicit-ref push uses this OID as its source, so a concurrent local
    // commit and repository push rules cannot change what gets published.
    let Some(desired) = head_oid(repo) else {
        return PushOutcome::Refused(
            "could not resolve HEAD to publish, so nothing was pushed.".to_string(),
        );
    };

    let result =
        super::remote::push_ref(repo, &remote, &desired, &reference, cancel, timeout).await;
    if matches!(&result, Err(network::Error::Cancelled)) {
        return PushOutcome::Unknown {
            branch,
            upstream,
            error: "publication was cancelled while git push was in flight; the remote may have accepted the update before the process was stopped"
                .to_string(),
        };
    }
    let result = result.map_err(|error| error.to_string());

    // A success reply is still only a claim until the remote ref is observed.
    // An error can likewise arrive after the update landed, so both paths use
    // the same fresh read and differ only when the remote stayed unchanged.
    let before = Ok(Some(live_upstream_tip));
    let after = super::remote::remote_oid(repo, &remote, &reference, cancel, timeout)
        .await
        .map_err(|error| error.to_string());
    let confirmation_error = match &after {
        Ok(Some(actual)) => format!(
            "git push reported success, but {upstream} points to {actual} instead of the frozen apply commit {desired}; whether it was published is unknown"
        ),
        Ok(None) => format!(
            "git push reported success, but {upstream} no longer exists; whether the frozen apply commit was published is unknown"
        ),
        Err(error) => format!(
            "git push reported success, but {upstream} could not be confirmed afterward: {error}; whether the frozen apply commit was published is unknown"
        ),
    };
    match (result, super::remote::classify(&before, &desired, &after)) {
        (_, super::remote::UpdateAfterError::Landed) => PushOutcome::Pushed {
            branch,
            upstream,
            remote,
            oid: desired,
        },
        (Err(error), super::remote::UpdateAfterError::Rejected) => PushOutcome::Failed(error),
        (Err(error), super::remote::UpdateAfterError::Unknown) => PushOutcome::Unknown {
            branch,
            upstream,
            error,
        },
        (Ok(_), _) => PushOutcome::Unknown {
            branch,
            upstream,
            error: confirmation_error,
        },
    }
}

#[cfg(test)]
#[path = "push/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "push/cancellation_tests.rs"]
mod cancellation_tests;

#[cfg(test)]
#[path = "push/publication_tests.rs"]
mod publication_tests;
