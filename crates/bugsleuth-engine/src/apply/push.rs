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

use super::observed::git;

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
    /// git ran and rejected it. Left for the user: the fix is a fetch and a
    /// rebase, or a force-push, and neither is this tool's to choose.
    Failed(String),
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
pub(super) fn push(repo: &Path, commits: usize, attributed: &[String]) -> PushOutcome {
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
    match git(repo, &["-c", "push.default=upstream", "push"]) {
        Ok(_) => PushOutcome::Pushed { branch, upstream },
        Err(error) => PushOutcome::Failed(error.to_string()),
    }
}

#[cfg(test)]
#[path = "push/tests.rs"]
mod tests;
