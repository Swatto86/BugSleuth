//! Tagging a pushed apply, so the repository's own CI cuts the release.
//!
//! This exists because the last manual step after "fix the bugs and publish
//! them" is always the same one: bump the patch, tag it, push the tag, wait for
//! the workflow. None of that needs a person, but all of it needs to be timid,
//! because a tag is the trigger for someone else's release pipeline.
//!
//! Two rules shape everything below:
//!
//! - **Only after the commits are on the remote.** A tag pushed ahead of its
//!   commits starts a build of a ref the runner cannot fetch, and the release it
//!   publishes — if it publishes one — is of the wrong tree. So this runs only
//!   after a successful push, and never in its place.
//! - **Never a name that already exists.** Reusing a tag means force-pushing it,
//!   which silently repoints a published release at a different commit. A
//!   collision stops here instead.
//!
//! The version is not invented. It is read from the repository's own most recent
//! `v*` tag and the patch is bumped, because a release of bug fixes is a patch
//! release by definition. A repository with no such tag has no scheme to follow,
//! so nothing is guessed for it.

use std::path::Path;
use std::time::Duration;

use crate::cancel::Cancel;

use super::{network, observed::git};

/// What became of the request to tag. Like [`super::PushOutcome`], every variant
/// is a normal outcome — the fixes are committed and published either way, so a
/// refusal is reported rather than raised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagOutcome {
    /// The setting is off, so nothing was attempted.
    NotRequested,
    /// Asked for, but the commits never reached the remote. A tag on a ref the
    /// runner cannot fetch is a broken release, not a release.
    NotPushed,
    /// Asked for and declined, for the reason given.
    Refused(String),
    Tagged {
        tag: String,
        remote: String,
    },
    /// git ran and the remote tag is confirmed still absent: a genuine rejection.
    /// Left for the user, exactly as a rejected push is.
    Failed(String),
    /// The tag push errored and the remote could not be confirmed either way.
    /// The release may already have started, so this is never described as a
    /// definite failure and the local tag is kept rather than deleted.
    Unknown {
        tag: String,
        remote: String,
        error: String,
    },
}

/// The next patch version after `latest`, or `None` if it is not a version this
/// understands.
///
/// Deliberately strict. `v1.2.3` and nothing else: no suffixes, no two-part
/// versions, no build metadata. A tag this cannot parse means the repository
/// names its releases some other way, and the honest answer to a scheme we do
/// not know is to decline rather than to publish `v1.2.4` into a history of
/// `2024.11-rc2`.
fn next_patch(latest: &str) -> Option<String> {
    let digits = latest.strip_prefix('v')?;
    let mut parts = digits.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("v{major}.{minor}.{}", patch.checked_add(1)?))
}

/// Whether this may be tagged at all, given what the push did.
///
/// Split from the git calls so the guards can be tested without a remote, the
/// same way [`super::push`] splits its own.
fn blocked(pushed: bool) -> Option<TagOutcome> {
    if !pushed {
        return Some(TagOutcome::NotPushed);
    }
    None
}

/// Tag the pushed commits and publish the tag, if it may be published.
///
/// `remote` is the exact remote the push used, threaded through from the push
/// itself rather than re-derived by splitting an `upstream` display string on
/// '/' — a remote name may itself contain a slash, and splitting sent the tag to
/// the wrong place. If the commits went to a fork, so does the tag.
pub(super) async fn tag(
    repo: &Path,
    pushed: bool,
    remote: &str,
    cancel: &Cancel,
    timeout: Duration,
) -> TagOutcome {
    if let Some(stop) = blocked(pushed) {
        return stop;
    }

    if remote.is_empty() {
        return TagOutcome::Refused(
            "the push did not report which remote it used, so nothing was tagged.".to_string(),
        );
    }

    // `--sort=-v:refname` orders by version rather than by string, so v1.10.0
    // sorts after v1.9.0 instead of before it. `--merged HEAD` keeps it to tags
    // in this branch's history: a tag on an unrelated branch is not the version
    // these fixes follow.
    let latest = match git(
        repo,
        &[
            "tag",
            "--list",
            "v*",
            "--sort=-v:refname",
            "--merged",
            "HEAD",
        ],
    ) {
        Ok(list) => list.lines().next().unwrap_or("").trim().to_string(),
        Err(error) => return TagOutcome::Failed(error.to_string()),
    };
    if latest.is_empty() {
        return TagOutcome::Refused(
            "this branch has no `v*` release tag to follow, so there is no version scheme to \
             continue. The commits are pushed — tag the first release by hand, and this will \
             follow it after that."
                .to_string(),
        );
    }
    let Some(next) = next_patch(&latest) else {
        return TagOutcome::Refused(format!(
            "{latest} is not a `vMAJOR.MINOR.PATCH` tag, so the next version cannot be worked out \
             from it. The commits are pushed — tag this release by hand."
        ));
    };

    // A name already in use is where this stops. Reusing it means moving a tag
    // that a published release already points at, and `git tag` refuses that
    // without `--force` — which is not a flag this gets to pass.
    if git(
        repo,
        &["rev-parse", "--verify", "--quiet", &format!("{next}^{{}}")],
    )
    .is_ok()
    {
        return TagOutcome::Refused(format!(
            "{next} already exists, so nothing was tagged. Moving a tag repoints whatever release \
             was built from it at a different commit."
        ));
    }

    // The same name may exist on the remote without existing locally — another
    // machine may have tagged it. Publishing over it would repoint a release, so
    // this is refused too. A remote that cannot be read is not treated as a
    // collision; the local check above already ran.
    let reference = format!("refs/tags/{next}");
    let remote_before =
        match super::remote::remote_oid(repo, remote, &reference, cancel, timeout).await {
            Ok(oid) => Ok(oid),
            Err(network::Error::Failed(error)) => Err(error),
            Err(network::Error::Cancelled) => {
                return TagOutcome::Refused(
                    "publication was cancelled before the release tag was created".to_string(),
                );
            }
        };
    if let Ok(Some(_)) = &remote_before {
        return TagOutcome::Refused(format!(
            "{next} already exists on {remote}, so nothing was tagged. Moving a published tag \
             repoints whatever release was built from it."
        ));
    }

    if cancel.stopped() {
        return TagOutcome::Refused(
            "publication was cancelled before the release tag was created".to_string(),
        );
    }

    // Annotated, not lightweight: `git describe` and most release tooling only
    // see annotated tags, and a release trigger that half the tooling cannot
    // find is worse than no tag.
    if let Err(error) = git(
        repo,
        &["tag", "-a", &next, "-m", &format!("{next}: bug fixes")],
    ) {
        return TagOutcome::Failed(error.to_string());
    }

    publish_tag(repo, remote, &next, remote_before, cancel, timeout).await
}

async fn publish_tag(
    repo: &Path,
    remote: &str,
    tag: &str,
    remote_before: Result<Option<String>, String>,
    cancel: &Cancel,
    timeout: Duration,
) -> TagOutcome {
    let desired = match git(repo, &["rev-parse", tag]) {
        Ok(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => {
            let _ = git(repo, &["tag", "-d", tag]);
            return TagOutcome::Failed(format!("could not resolve the new tag {tag}"));
        }
    };

    // By name, never `--tags`: bulk publication can expose private backup tags.
    match network::git(repo, &["push", remote, tag], cancel, timeout).await {
        Ok(_) => TagOutcome::Tagged {
            tag: tag.to_string(),
            remote: remote.to_string(),
        },
        Err(network::Error::Cancelled) => TagOutcome::Unknown {
            tag: tag.to_string(),
            remote: remote.to_string(),
            error: "publication was cancelled while the tag push was in flight; the remote may have accepted the tag before the process was stopped"
                .to_string(),
        },
        Err(error) => {
            // A push error only means the client got no success reply. The tag
            // may already be on the remote and the release already running, so
            // re-read the ref before deciding — and never delete the local tag
            // unless the remote is confirmed to have stayed unchanged.
            let reference = format!("refs/tags/{tag}");
            let after = super::remote::remote_oid(repo, remote, &reference, cancel, timeout)
                .await
                .map_err(|error| error.to_string());
            match super::remote::classify(&remote_before, &desired, &after) {
                super::remote::UpdateAfterError::Landed => TagOutcome::Tagged {
                    tag: tag.to_string(),
                    remote: remote.to_string(),
                },
                super::remote::UpdateAfterError::Rejected => {
                    // Confirmed absent on the remote: safe to remove the local
                    // tag so a retry is not blocked by the collision check.
                    let cleanup = git(repo, &["tag", "-d", tag]);
                    match cleanup {
                        Ok(_) => TagOutcome::Failed(error.to_string()),
                        Err(cleanup_error) => TagOutcome::Failed(format!(
                            "{error}; and the local tag {tag} could not be removed afterward: \
                             {cleanup_error}"
                        )),
                    }
                }
                super::remote::UpdateAfterError::Unknown => TagOutcome::Unknown {
                    tag: tag.to_string(),
                    remote: remote.to_string(),
                    error: error.to_string(),
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "tag/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tag/test_support.rs"]
mod test_support;
