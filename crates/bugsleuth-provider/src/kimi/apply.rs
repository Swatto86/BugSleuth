//! Refusing to hand a fix prompt to Kimi in the real repository.
//!
//! Kimi has no per-invocation write confinement. `--yolo` and `--auto` only
//! loosen approvals, and there is no tool allowlist — so nothing here can stop
//! an apply from doing more than editing files, and the prompt it would be
//! given is model-produced prose derived from an untrusted repository.
//!
//! A sweep can be confined by pointing it at a disposable worktree. An apply
//! cannot: editing the real checkout is the entire point of it.
//!
//! This is the same refusal Kilo carries, for the same reason.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;

use super::VENDOR;

/// Apply the fixes described in `prompt`, returning the model's own account.
pub async fn apply(
    _repo: &Path,
    _model: &str,
    _effort: &str,
    _prompt: &str,
    _timeout: Duration,
) -> Result<String, ProviderError> {
    Err(ProviderError::CapabilityUnavailable {
        vendor: VENDOR,
        capability: "apply",
        reason: "it has no per-invocation tool or write confinement, so BugSleuth cannot safely grant it write access to your checkout. Apply the generated handoff manually, or with a provider that can be confined."
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The refusal must come before any CLI is discovered or launched.
    ///
    /// A refusal that first spawned the tool would already have handed it the
    /// prompt, which is the thing being refused.
    #[tokio::test]
    async fn apply_is_refused_and_says_why() {
        let error = apply(
            Path::new("."),
            "kimi-k3",
            "",
            "fix everything",
            Duration::from_secs(1),
        )
        .await
        .expect_err("Kimi cannot be granted write access");
        let shown = error.to_string();
        assert!(shown.contains("kimi"), "{shown}");
        assert!(
            shown.contains("confinement"),
            "the refusal does not say why: {shown}"
        );
    }
}
