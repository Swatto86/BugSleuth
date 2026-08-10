//! Reconciling a git push that failed to acknowledge.
//!
//! A non-zero `git push` means only that the client did not *receive* a success
//! reply. The remote may already have applied the ref update before the
//! connection broke — a tag push in particular can land, start the release
//! workflow, and only then have its acknowledgement lost. Treating that as
//! "nothing happened" both misreports the release and, for a tag, deletes the
//! one local record of it.
//!
//! So after an ambiguous failure the caller re-reads the remote ref and asks
//! this module what actually happened: it landed, it was rejected, or — when the
//! remote cannot be read — it is genuinely unknown and must not be retried.

use std::path::Path;
use std::time::Duration;

use crate::cancel::Cancel;

use super::network;

/// What a remote ref shows after a push returned an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdateAfterError {
    /// The desired object is on the remote: the push landed despite the error.
    Landed,
    /// The remote is confirmed unchanged: the push really was rejected.
    Rejected,
    /// The remote could not be read, or changed to something else. The outcome
    /// is indeterminate and must not be retried — a retry could duplicate an
    /// irreversible action.
    Unknown,
}

/// Decide an ambiguous push's real outcome from the remote ref before and after.
///
/// `before`/`after` are the object IDs the remote reported for the ref (`None`
/// when the ref did not exist), or an error string when the remote could not be
/// read. `desired` is the object ID the push was trying to place there.
pub(super) fn classify(
    before: &Result<Option<String>, String>,
    desired: &str,
    after: &Result<Option<String>, String>,
) -> UpdateAfterError {
    match (before, after) {
        (_, Ok(Some(oid))) if oid == desired => UpdateAfterError::Landed,
        (Ok(before), Ok(after)) if before == after => UpdateAfterError::Rejected,
        _ => UpdateAfterError::Unknown,
    }
}

/// Publish one exact object to one exact remote ref.
///
/// The explicit refspec ignores repository `remote.*.push` rules, while the two
/// negative options prevent configured tag-following or submodule publication
/// from broadening this irreversible operation.
pub(super) async fn push_ref(
    repo: &Path,
    remote: &str,
    oid: &str,
    reference: &str,
    cancel: &Cancel,
    timeout: Duration,
) -> Result<String, network::Error> {
    let refspec = format!("{oid}:{reference}");
    network::git(
        repo,
        &[
            "push",
            "--no-follow-tags",
            "--recurse-submodules=no",
            "--",
            remote,
            &refspec,
        ],
        cancel,
        timeout,
    )
    .await
}

/// The object ID a remote reports for `reference`, or `None` if it has no such
/// ref. An empty successful listing is a definite absence; a command or parse
/// failure is an error, never silently read as absence.
///
/// The peeled `^{}` line an annotated tag adds is skipped, so a tag ref resolves
/// to its single tag-object ID rather than being seen as two objects.
pub(super) async fn remote_oid(
    repo: &Path,
    remote: &str,
    reference: &str,
    cancel: &Cancel,
    timeout: Duration,
) -> Result<Option<String>, network::Error> {
    let listing = network::git(repo, &["ls-remote", remote, reference], cancel, timeout).await?;
    let mut ids = listing.lines().filter_map(|line| {
        let mut parts = line.split_whitespace();
        let oid = parts.next()?;
        let name = parts.next().unwrap_or("");
        if name.ends_with("^{}") || oid.is_empty() {
            return None;
        }
        Some(oid.to_string())
    });
    let first = ids.next();
    if ids.next().is_some() {
        return Err(network::Error::Failed(format!(
            "{remote} reported more than one object for {reference}"
        )));
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desired_object_on_the_remote_is_a_landing_whatever_was_there_before() {
        let before = Ok(Some("aaa".to_string()));
        let after = Ok(Some("bbb".to_string()));
        assert_eq!(classify(&before, "bbb", &after), UpdateAfterError::Landed);
        // Even when the ref did not exist before.
        assert_eq!(
            classify(&Ok(None), "bbb", &Ok(Some("bbb".to_string()))),
            UpdateAfterError::Landed
        );
    }

    #[test]
    fn an_unchanged_remote_is_a_genuine_rejection() {
        assert_eq!(
            classify(&Ok(Some("aaa".into())), "bbb", &Ok(Some("aaa".into()))),
            UpdateAfterError::Rejected
        );
        assert_eq!(
            classify(&Ok(None), "bbb", &Ok(None)),
            UpdateAfterError::Rejected
        );
    }

    #[test]
    fn a_remote_that_moved_elsewhere_or_could_not_be_read_is_unknown() {
        // Changed to a third object neither before nor desired.
        assert_eq!(
            classify(&Ok(Some("aaa".into())), "bbb", &Ok(Some("ccc".into()))),
            UpdateAfterError::Unknown
        );
        // The remote could not be read after the failure.
        assert_eq!(
            classify(&Ok(Some("aaa".into())), "bbb", &Err("offline".into())),
            UpdateAfterError::Unknown
        );
    }
}
