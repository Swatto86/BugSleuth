//! Keeping this tool's name out of someone else's history.
//!
//! Split from `observed.rs` at the hard line cap, and the seam is real: that
//! file answers "what did the model change", this one answers "and did it sign
//! the work". They share only git.
//!
//! The rule is that BugSleuth does not attribute an AI in a repository it was
//! asked to fix. Three things enforce it, because one is not enough: the CLI is
//! told not to add a trailer, the prompt says so too, and what actually landed
//! is read back and repaired. The first two are requests; this is the check.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use crate::cancel::Cancel;

use super::Baseline;
use super::observed::{git, git_with_env, range_since};

mod published;

/// Rewrite the commits this apply created, without their tool trailers.
///
/// The setting passed to the CLI removes the cause for one vendor; this removes
/// it for all of them, including a model that types the trailer out by hand
/// despite being asked not to. The user said plainly that this tool must not
/// attribute an AI in their repository, and an instruction is not a guarantee.
///
/// **Only ever commits this apply made, and only while they are local.** The
/// range is `base..HEAD`, where `base` is where the repository stood before the
/// model touched it, so nothing older can be reached. If any of those commits
/// has already reached a remote, rewriting would fork the published history —
/// so it is refused and reported instead, which is the honest outcome.
///
/// Returns the subjects it rewrote.
pub(super) async fn strip_attribution(
    repo: &Path,
    base: &Baseline,
    cancel: &Cancel,
    timeout: Duration,
) -> anyhow::Result<Vec<String>> {
    let branch = git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|b| b.trim().to_string())
        .unwrap_or_default();
    if branch.is_empty() {
        anyhow::bail!("HEAD is detached, so there is no branch to move");
    }

    // The commits this apply created. Past a real base that is `base..HEAD`;
    // from an unborn start it is the whole of HEAD, whose first commit is a
    // root with no parent. An unborn start that committed nothing has no range
    // and nothing to strip — and any failure to establish that is an error, not
    // an empty range that would quietly skip the whole guard.
    let Some(_) = range_since(repo, base)? else {
        return Ok(vec![]);
    };
    let expected_head = git(repo, &["rev-parse", "HEAD"])?.trim().to_string();
    strip_attribution_at(repo, base, &branch, &expected_head, cancel, timeout).await
}

async fn strip_attribution_at(
    repo: &Path,
    base: &Baseline,
    branch: &str,
    expected_head: &str,
    cancel: &Cancel,
    timeout: Duration,
) -> anyhow::Result<Vec<String>> {
    let range = match base {
        Baseline::Commit(base) => format!("{base}..{expected_head}"),
        Baseline::Unborn => expected_head.to_string(),
    };
    let ids: Vec<String> = git(repo, &["rev-list", "--topo-order", "--reverse", &range])?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    // Nothing to do is the common case, and it must cost nothing: rewriting the
    // range unconditionally would give every commit of every apply a new hash
    // for no reason - churn the user would notice and could not explain. A
    // message that cannot be read is an error, never an uncredited commit: a
    // credited commit missed here would be pushed with its trailer intact.
    let mut any_credited = false;
    for id in &ids {
        if message_of(repo, id)?.lines().any(credits_a_tool) {
            any_credited = true;
            break;
        }
    }
    if !any_credited {
        return Ok(vec![]);
    }
    refuse_if_published(repo, &ids, cancel, timeout).await?;

    // Parent-before-child order makes every rewritten parent available when a
    // child is recreated. Parents outside this apply's range retain their
    // original ids; parents inside it are replaced through this map.
    let mut rewritten_by_id: HashMap<String, String> = HashMap::new();
    let mut rewritten = Vec::new();
    for id in &ids {
        let parents = git(repo, &["show", "-s", "--format=%P", id])?
            .split_whitespace()
            .map(|parent| {
                rewritten_by_id
                    .get(parent)
                    .cloned()
                    .unwrap_or_else(|| parent.to_string())
            })
            .collect::<Vec<_>>();
        let (new, stripped) = rewrite_one(repo, id, &parents)?;
        if let Some(subject) = stripped {
            rewritten.push(subject);
        }
        rewritten_by_id.insert(id.clone(), new);
    }

    // `update-ref` with the frozen old value, so a branch that moved while this
    // ran is left alone rather than clobbered.
    // `any_credited` above guarantees expected HEAD was in the enumerated
    // range; retain an explicit error if Git ever violates that assumption.
    let new_head = rewritten_by_id
        .get(expected_head)
        .ok_or_else(|| anyhow::anyhow!("expected HEAD was not rewritten"))?;
    git(
        repo,
        &[
            "update-ref",
            &format!("refs/heads/{branch}"),
            new_head,
            expected_head,
        ],
    )?;
    Ok(rewritten)
}

/// Refuse the whole rewrite if any of these commits has left this machine.
///
/// Forking published history to tidy a trailer is a worse outcome than the
/// trailer, so this is where the guarantee stops and the report takes over.
async fn refuse_if_published(
    repo: &Path,
    ids: &[String],
    cancel: &Cancel,
    timeout: Duration,
) -> anyhow::Result<()> {
    published::refuse(repo, ids, cancel, timeout).await
}

fn message_of(repo: &Path, id: &str) -> anyhow::Result<String> {
    git(repo, &["log", "-1", "--format=%B", id])
}

/// Re-commit one commit onto all of its rewritten parents without its trailers.
/// An empty slice recreates the initial commit of an unborn repository as a
/// root; each additional `-p` preserves a merge edge.
///
/// Returns the new id, and the subject when a trailer was actually removed.
fn rewrite_one(
    repo: &Path,
    id: &str,
    parents: &[String],
) -> anyhow::Result<(String, Option<String>)> {
    let message = message_of(repo, id)?;
    let cleaned = without_trailers(&message);
    let tree = git(repo, &["rev-parse", &format!("{id}^{{tree}}")])?
        .trim()
        .to_string();

    let cleaned = cleaned.trim_end();
    let mut args: Vec<&str> = vec!["commit-tree", &tree];
    for parent in parents {
        args.push("-p");
        args.push(parent);
    }
    args.push("-m");
    args.push(cleaned);
    let new = git_with_env(repo, &args, &authorship_of(repo, id)?)?
        .trim()
        .to_string();

    // Asked of the lines, not of the text. Comparing `cleaned != message`
    // called every commit rewritten, because git's `%B` ends with a newline the
    // cleaned copy normalises away - so a commit that never carried a trailer
    // was reported as having had one removed.
    let subject = message.lines().any(credits_a_tool).then(|| {
        message
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    });
    Ok((new, subject))
}

/// The original author and committer, as environment for `commit-tree`.
///
/// Carried across by hand because `commit-tree` would otherwise stamp the
/// rewrite with this process's identity and the time of day - quietly rewriting
/// who did the work, which is the very thing this exists to stop.
fn authorship_of(repo: &Path, id: &str) -> anyhow::Result<Vec<(String, String)>> {
    let who = git(
        repo,
        &[
            "log",
            "-1",
            "--format=%an%x00%ae%x00%aD%x00%cn%x00%ce%x00%cD",
            id,
        ],
    )?;
    let fields: Vec<&str> = who.trim_end().split('\u{0}').collect();
    match fields.as_slice() {
        [an, ae, ad, cn, ce, cd] => Ok(vec![
            ("GIT_AUTHOR_NAME".into(), (*an).to_string()),
            ("GIT_AUTHOR_EMAIL".into(), (*ae).to_string()),
            ("GIT_AUTHOR_DATE".into(), (*ad).to_string()),
            ("GIT_COMMITTER_NAME".into(), (*cn).to_string()),
            ("GIT_COMMITTER_EMAIL".into(), (*ce).to_string()),
            ("GIT_COMMITTER_DATE".into(), (*cd).to_string()),
        ]),
        _ => anyhow::bail!("could not read the author of {id}"),
    }
}

/// A commit message with any tool authorship trailer removed.
fn without_trailers(message: &str) -> String {
    let kept: Vec<&str> = message
        .lines()
        .filter(|line| !credits_a_tool(line))
        .collect();
    // A trailer sitting alone under a blank line leaves a trailing gap behind
    // it; git strips that on commit, but the comparison above is on the text.
    kept.join("\n").trim_end().to_string() + "\n"
}

/// Subjects of new commits whose message carries a tool authorship trailer.
///
/// `%x1e` between commits and `%x00` between the subject and the body, because
/// a commit message contains blank lines and any separator that occurs in prose
/// eventually splits one in half.
pub(super) fn attributed_since(repo: &Path, base: &Baseline) -> anyhow::Result<Vec<String>> {
    let Some(range) = range_since(repo, base)? else {
        return Ok(vec![]);
    };
    let log = git(repo, &["log", "--format=%s%x00%b%x1e", &range])?;
    Ok(attributed(&log))
}

/// The subjects of commits that credit a tool for the work.
///
/// Deliberately narrow. A `Co-Authored-By` naming a colleague is a normal thing
/// to want; one naming the CLI that BugSleuth just drove is attribution nobody
/// asked for, in a repository whose owner may have a rule against it — and it
/// arrives by default, which is why an instruction alone cannot be trusted.
pub(super) fn attributed(log: &str) -> Vec<String> {
    log.split('\u{1e}')
        .filter_map(|record| {
            let (subject, body) = record.trim_start().split_once('\u{0}')?;
            let credited = credits_a_tool(subject) || body.lines().any(credits_a_tool);
            credited.then(|| subject.trim().to_string())
        })
        .filter(|subject| !subject.is_empty())
        .collect()
}

/// Whether one line of a commit message credits a tool for the work.
///
/// The single definition, used both to report these and to strip them. Two
/// copies of this rule would eventually disagree, and the disagreement that
/// matters is silent: a line the report names and the rewrite leaves behind,
/// or the reverse.
fn credits_a_tool(line: &str) -> bool {
    // Names, and the domains they sign from: a trailer reading
    // `Assistant <noreply@anthropic.com>` names no tool the first half of this
    // list would catch, and it is the same attribution.
    const TOOLS: [&str; 9] = [
        "claude",
        "codex",
        "kilo",
        "copilot",
        "chatgpt",
        "gemini",
        "anthropic.com",
        "openai.com",
        "noreply@google.com",
    ];
    let line = line.trim().to_lowercase();
    (line.starts_with("co-authored-by:") || line.contains("generated with"))
        && TOOLS.iter().any(|tool| line.contains(tool))
}

#[cfg(test)]
#[path = "attribution/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "attribution/safety_tests.rs"]
mod safety_tests;
