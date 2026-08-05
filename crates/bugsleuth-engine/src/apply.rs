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
use std::process::Command;
use std::time::Duration;

use crate::sweep::Vendor;

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

    Ok(ApplyReport {
        text,
        changed_files: changed_since(repo, base.as_deref()),
        commits: commits_since(repo, base.as_deref()),
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

/// Repo-relative paths differing from `base`, committed or not.
///
/// Two questions, because one command answers neither on its own: `git diff`
/// knows about tracked files only, and a new file the model created is exactly
/// the kind of change most worth seeing.
fn changed_since(repo: &Path, base: Option<&str>) -> Vec<String> {
    let mut files: Vec<String> = match base {
        Some(base) => git(repo, &["diff", "--name-only", base])
            .map(|out| lines(&out))
            .unwrap_or_default(),
        // No commit to compare against — a repository with no history yet. The
        // working tree is all there is.
        None => git(repo, &["status", "--porcelain"])
            .map(|out| dirty_files(&out))
            .unwrap_or_default(),
    };
    files.extend(
        git(repo, &["ls-files", "--others", "--exclude-standard"])
            .map(|out| lines(&out))
            .unwrap_or_default(),
    );
    files.sort();
    files.dedup();
    theirs(files)
}

fn commits_since(repo: &Path, base: Option<&str>) -> usize {
    let Some(base) = base else { return 0 };
    git(repo, &["rev-list", "--count", &format!("{base}..HEAD")])
        .ok()
        .and_then(|out| out.trim().parse().ok())
        .unwrap_or(0)
}

/// Where BugSleuth's own throwaway checkouts live inside a reviewed repository.
///
/// A proof attempt that was killed rather than dropped leaves one behind, and
/// git reports it as untracked. Judged as the user's uncommitted work it would
/// refuse every apply — with advice to "commit or stash" litter this tool left —
/// until someone deleted a directory by hand. It is not their work, so it is not
/// counted as theirs, in the guard or in the list of what changed.
const OURS: &str = ".bugsleuth-worktrees/";

fn theirs(files: Vec<String>) -> Vec<String> {
    files
        .into_iter()
        .filter(|path| !path.starts_with(OURS))
        .collect()
}

/// The paths in `git status --porcelain` output.
///
/// The status letters occupy the first two columns and a space follows, so the
/// name starts at column three. A rename is reported as `old -> new`; the new
/// name is the one that exists, and it is what a reader needs.
fn dirty_files(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|name| {
            let name = name.rsplit(" -> ").next().unwrap_or(name);
            name.trim().trim_matches('"').replace('\\', "/")
        })
        .filter(|name| !name.is_empty())
        .collect()
}

fn lines(out: &str) -> Vec<String> {
    out.lines()
        .map(|line| line.trim().replace('\\', "/"))
        .filter(|line| !line.is_empty())
        .collect()
}

/// A few names and a count, rather than a wall of paths in an error message.
fn summarise(files: &[String]) -> String {
    const SHOWN: usize = 5;
    let head = files
        .iter()
        .take(SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    match files.len().checked_sub(SHOWN) {
        Some(rest) if rest > 0 => format!("{head} and {rest} more"),
        _ => head,
    }
}

fn git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = bugsleuth_verify::hide_console_window(&mut Command::new("git"))
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| anyhow::anyhow!("could not run git — is it installed and on PATH? ({e})"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&"?"),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dirty_tree_is_read_out_of_git_status_whatever_the_status_letters_are() {
        let porcelain = " M src/main.rs\n?? new.txt\nA  added.rs\nR  old.rs -> renamed.rs\n";
        let dirty = dirty_files(porcelain);
        assert_eq!(dirty, ["src/main.rs", "new.txt", "added.rs", "renamed.rs"]);
    }

    #[test]
    fn a_clean_tree_reads_as_clean_rather_than_as_one_empty_path() {
        // The guard is `is_empty()`, so a blank line surviving the parse would
        // refuse to apply anything on a perfectly clean repository.
        assert!(dirty_files("").is_empty());
        assert!(dirty_files("\n\n").is_empty());
        // A status line with the letters and nothing after them. The name is
        // then the empty string, and one of those in the list is enough to
        // refuse to apply anything on a repository that is perfectly clean.
        assert!(dirty_files("?? \n").is_empty());
    }

    #[test]
    fn bugsleuths_own_leftovers_are_not_treated_as_the_users_uncommitted_work() {
        // A proof attempt that was killed leaves a worktree behind, and git
        // calls it untracked. Counting it would refuse every apply — telling
        // the user to commit or stash a directory this tool created — until
        // they deleted it by hand.
        let mixed = vec![
            ".bugsleuth-worktrees/prove-1/src/main.rs".to_string(),
            "src/real.rs".to_string(),
        ];
        assert_eq!(theirs(mixed), ["src/real.rs"]);
        // And a repository whose only "changes" are ours reads as clean.
        assert!(theirs(vec![".bugsleuth-worktrees/x".to_string()]).is_empty());
    }

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

    #[test]
    fn the_refusal_names_a_few_files_rather_than_all_of_them() {
        let many: Vec<String> = (0..9).map(|n| format!("f{n}.rs")).collect();
        let text = summarise(&many);
        assert!(text.contains("f0.rs"));
        assert!(text.contains("and 4 more"), "{text}");
        assert!(!text.contains("f8.rs"));
        assert_eq!(summarise(&["one.rs".to_string()]), "one.rs");
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
