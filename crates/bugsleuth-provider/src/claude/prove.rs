//! Asking a model to demonstrate a defect with a failing test.
//!
//! This is the step that turns an assertion into evidence. The model is given
//! write and test-execution access — but only inside a throwaway git worktree
//! that the caller created, never a real working tree.
//!
//! Note what the model is asked *not* to do: fix the defect. A proof attempt
//! that also repairs the code produces a passing test and proves nothing, and it
//! is the most natural thing in the world for a coding agent to do unprompted.

use std::path::Path;
use std::time::Duration;

use bugsleuth_domain::{ProofClaim, proof_schema};

use super::{ProviderError, Run, invoke};

/// Tools a proof attempt needs. Wider than a sweep's, because writing a test and
/// running it is the entire task.
///
/// **`Bash` is on this list, so this is not a network restriction and must not
/// be read as one.** The comment here used to end "and still no network
/// access", which was false: denying `WebFetch` and `WebSearch` beside a shell
/// stops nothing — `curl` is one command away — and the reviewed repository is
/// untrusted input that can address the model directly. Found by BugSleuth
/// reviewing itself, and it is the "comment describes behaviour the code does
/// not implement" class landing on a security claim, which is the worst place
/// for it: a reader checking whether proving is safe would have stopped here.
///
/// The denials stay because they remove the two most convenient paths and cost
/// nothing. What is actually true about proving — that it runs the reviewed
/// repository's own build and test commands with the user's permissions and no
/// sandbox — is stated in `PROVING_EXECUTION_WARNING` and printed in the report
/// whenever proof is attempted.
const PROVE_TOOLS: &str = "Read,Glob,Grep,Edit,Write,Bash";
const PROVE_DENIED: &str = "WebFetch,WebSearch";

pub struct ProveRequest<'a> {
    /// The worktree the model may write to. Never a real working tree.
    pub worktree: &'a Path,
    pub model: &'a str,
    /// Description of the defect, assembled by the caller.
    pub brief: &'a str,
    pub timeout: Duration,
    pub max_turns: u32,
    pub binary: Option<&'a str>,
    pub api_key: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct ProveResult {
    /// The model's own account. Believed only after the tests are re-run.
    pub claim: ProofClaim,
    pub turns: Option<u32>,
    pub session_id: Option<String>,
}

pub async fn prove(request: ProveRequest<'_>) -> Result<ProveResult, ProviderError> {
    let outcome = invoke(Run {
        effort: "",
        repo: request.worktree,
        model: request.model,
        prompt: request.brief,
        schema: proof_schema(),
        allowed: PROVE_TOOLS,
        denied: PROVE_DENIED,
        max_turns: request.max_turns,
        timeout: request.timeout,
        binary: request.binary,
        api_key: request.api_key,
        resume: None,
    })
    .await?;

    let claim: ProofClaim = crate::json::structured(&outcome.result)?;
    Ok(ProveResult {
        claim,
        turns: outcome.num_turns,
        session_id: outcome.session_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proof_attempt_may_write_and_run_tests_but_not_reach_the_network() {
        for tool in ["Edit", "Write", "Bash", "Read"] {
            assert!(
                PROVE_TOOLS.split(',').any(|t| t == tool),
                "a proof attempt needs {tool}"
            );
        }
        for tool in ["WebFetch", "WebSearch"] {
            assert!(PROVE_DENIED.split(',').any(|t| t == tool));
            assert!(!PROVE_TOOLS.split(',').any(|t| t == tool));
        }
    }
}
