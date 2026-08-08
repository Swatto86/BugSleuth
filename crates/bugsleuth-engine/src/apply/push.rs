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

use super::{Baseline, observed::git};

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

/// Push the branch the apply committed on, if it may be pushed.
pub(super) fn push(
    repo: &Path,
    base: &Baseline,
    commits: usize,
    attributed: &[String],
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

    let location = format!("refs/heads/{branch}");
    let (remote, reference) = match git(
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
    }) {
        Ok(location) => location,
        Err(error) => {
            return PushOutcome::Refused(format!(
                "could not establish the live location of {upstream}: {error}. Nothing was pushed."
            ));
        }
    };

    let live_upstream_tip = match git(repo, &["ls-remote", &remote, &reference]).and_then(|value| {
        let mut ids = value
            .lines()
            .filter_map(|line| line.split_whitespace().next());
        let Some(id) = ids.next().filter(|id| !id.is_empty()) else {
            anyhow::bail!("the upstream ref does not exist");
        };
        if ids.next().is_some() {
            anyhow::bail!("git reported more than one upstream object ID");
        }
        Ok(id.to_string())
    }) {
        Ok(tip) => tip,
        Err(error) => {
            return PushOutcome::Refused(format!(
                "could not establish the live tip of {upstream}: {error}. Nothing was pushed."
            ));
        }
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

    // `push.default=upstream` for this invocation only, which is what makes a
    // bare `git push` mean "this branch, to its upstream, and nothing else".
    //
    // Without it the repository's own setting decides, and the old `matching`
    // default pushes *every* local branch that shares a name with one on the
    // remote — so applying a fix on one branch would publish however many
    // unrelated branches happened to be lying around. `-c` overrides it for
    // this command without touching the user's config.
    //
    // Still no force and no refspec of our own: a rejection is left rejected,
    // because recovering from one means a fetch and a rebase, or a force, and
    // a tool that picks either on your behalf is a tool that loses work.
    // The object a successful push would place on the remote branch.
    let desired = match git(repo, &["rev-parse", "HEAD"]) {
        Ok(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => {
            return PushOutcome::Refused(
                "could not resolve HEAD to publish, so nothing was pushed.".to_string(),
            );
        }
    };

    match git(repo, &["-c", "push.default=upstream", "push"]) {
        Ok(_) => PushOutcome::Pushed { branch, upstream },
        Err(error) => {
            // A push error only means the client got no success reply. The
            // update may already be on the remote, so re-read the ref before
            // calling it a failure.
            let before = Ok(Some(live_upstream_tip));
            let after = super::remote::remote_oid(repo, &remote, &reference);
            match super::remote::classify(&before, &desired, &after) {
                super::remote::UpdateAfterError::Landed => PushOutcome::Pushed { branch, upstream },
                super::remote::UpdateAfterError::Rejected => PushOutcome::Failed(error.to_string()),
                super::remote::UpdateAfterError::Unknown => PushOutcome::Unknown {
                    branch,
                    upstream,
                    error: error.to_string(),
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "push/tests.rs"]
mod tests;
