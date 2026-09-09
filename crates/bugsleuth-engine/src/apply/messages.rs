//! What to tell someone about a fix run that did not reach its end.
//!
//! Split from `apply.rs` at the hard line cap, on its own subject. These are
//! the only words a user gets about a run that was editing their repository
//! when it stopped, and the thing they all have to convey is the same: what is
//! already committed, what is recorded as done, and that continuing does not
//! mean paying for it again.

use std::path::Path;

use super::Baseline;
use super::journal::Journal;
use super::observed::{changed_since, summarise};

/// What to say when a run stops before it has fixed everything.
///
/// Names the count rather than the defects: the numbers are what decides
/// whether to wait for quota to reset or to stop here, and the report itself
/// already lists which defect is which.
pub(super) fn resumable(done: usize, total: usize) -> String {
    let remaining = total.saturating_sub(done);
    format!(
        "{done} of {total} defects are fixed and committed, and are recorded as done — \
         pressing Apply again continues at the {remaining} still outstanding rather than \
         starting over."
    )
}

/// The message for a run the user stopped.
pub(super) fn stopped_message(
    progress: &Journal,
    total: usize,
    repo: &Path,
    base: &Baseline,
) -> String {
    let done = progress.completed();
    if done == 0 {
        return cancelled_message().to_string();
    }
    // What git can see, because a stop between defects still leaves every
    // earlier fix committed and someone reading only "stopped" would not know
    // their repository had changed at all.
    let changed = changed_since(repo, base).unwrap_or_default();
    let seen = if changed.is_empty() {
        String::new()
    } else {
        format!(" {}", summarise(&changed))
    };
    format!("The fixes were stopped. {}{seen}", resumable(done, total))
}

/// The message for a run whose provider failed.
///
/// A quota limit is called out by name because it is the one cause where the
/// answer is to wait rather than to change anything — and it is the reason a
/// long fix run usually ends early.
pub(super) fn interrupted_message(
    error: &bugsleuth_provider::ProviderError,
    progress: &Journal,
    total: usize,
) -> String {
    let done = progress.completed();
    let cause = if error.is_transient() {
        "This looks like a rate limit or a temporary provider failure rather than a problem \
         with your repository, so waiting and resuming is usually all it takes. "
    } else {
        ""
    };
    if done == 0 {
        return format!("{cause}No defect was completed, so nothing is recorded as done.");
    }
    format!("{cause}{}", resumable(done, total))
}

pub(super) fn cancelled_message() -> &'static str {
    "the apply was stopped. The model was killed part-way through editing the repository — check `git status` and `git log` to see what it had already changed."
}
