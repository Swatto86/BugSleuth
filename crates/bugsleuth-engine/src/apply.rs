//! Applying the fix prompt to the repository it came from.
//!
//! Everything else in BugSleuth is careful never to modify the code it reads: a
//! sweep is read-only, a proof attempt gets a throwaway worktree. This is the
//! one deliberate exception, and it exists because the alternative — copy the
//! prompt, open another tool, paste it — is what everyone was doing anyway.
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

use crate::sweep::Vendor;

mod attribution;
mod observed;
use attribution::{attributed_since, strip_attribution};
use observed::{changed_since, commits_since, dirty_files, git, summarise, theirs};

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

    let dirty = theirs(dirty_files(&git(repo, &["status", "--porcelain"])?));
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
    let base = git(repo, &["rev-parse", "HEAD"]).ok().and_then(|out| {
        let id = out.trim().to_string();
        (!id.is_empty()).then_some(id)
    });

    let (vendor, model) = Vendor::parse(request.model);
    let attempt = match vendor {
        Vendor::Claude => {
            bugsleuth_provider::claude::apply(bugsleuth_provider::claude::ApplyRequest {
                repo,
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
                repo,
                model,
                request.effort,
                request.prompt,
                request.timeout,
            )
            .await
        }
        Vendor::Kilo => {
            bugsleuth_provider::kilo::apply(
                repo,
                model,
                request.effort,
                request.prompt,
                request.timeout,
            )
            .await
        }
    };

    // A failure is not "nothing happened". The invocation is killed on timeout
    // and can fail after the model has already rewritten half the tree, and an
    // error on its own would send someone away believing their repository was
    // untouched. Whatever git can see is named in the error too.
    let text = match attempt {
        Ok(text) => text,
        Err(error) => anyhow::bail!(failure_message(
            &error.to_string(),
            &changed_since(repo, base.as_deref())
        )),
    };

    // Attribution comes off before anything else is reported, so what the user
    // reads describes the repository as it now stands. Only commits this apply
    // created, only while they are still local — see `strip_attribution`.
    //
    // A failure here is not a failed apply: the fixes are real and already
    // committed. What is left is named instead, which is the whole difference
    // between a tool that quietly forks published history and one that says
    // "these two are yours to deal with".
    let (stripped, attributed) = match base.as_deref() {
        Some(base) => match strip_attribution(repo, base) {
            Ok(stripped) => (stripped, vec![]),
            Err(_) => (vec![], attributed_since(repo, Some(base))),
        },
        None => (vec![], vec![]),
    };

    Ok(ApplyReport {
        text,
        changed_files: changed_since(repo, base.as_deref()),
        commits: commits_since(repo, base.as_deref()),
        stripped,
        attributed,
    })
}

/// A vendor failure, plus whatever it had already done to the repository.
///
/// A failure is not "nothing happened". The invocation is killed on timeout and
/// can fail after the model has rewritten half the tree, so an error on its own
/// would send someone away believing their checkout was untouched — the same
/// "absence of a result read as a result" this whole project exists to stop.
fn failure_message(error: &str, changed: &[String]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_apply_still_says_what_it_had_already_changed() {
        // The timeout case: the CLI is killed, and everything it wrote before
        // that is still on disk. Reporting only the error would send someone
        // away believing their repository was untouched.
        let text = failure_message("the codex CLI timed out", &["src/a.rs".to_string()]);
        assert!(text.contains("timed out"));
        assert!(text.contains("1 file had already changed"), "{text}");
        assert!(text.contains("src/a.rs"));
        assert!(text.contains("git status"));

        // And when nothing changed, it says so rather than staying silent —
        // "the run failed" alone leaves the reader guessing about their tree.
        let clean = failure_message("the kilo CLI exited with code 1", &[]);
        assert!(clean.contains("no files changed"), "{clean}");
    }

    #[tokio::test]
    async fn a_repository_without_git_is_refused_before_anything_is_spent() {
        let dir = std::env::temp_dir().join(format!("bugsleuth-apply-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let error = apply(ApplyRequest {
            repo: &dir,
            model: "haiku",
            effort: "",
            prompt: "fix it",
            timeout: Duration::from_secs(1),
            max_turns: 1,
        })
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(error.contains("not a git repository"), "{error}");
    }
}
