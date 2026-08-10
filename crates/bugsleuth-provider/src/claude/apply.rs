//! Handing the fix prompt to Claude, with write access to the real repository.
//!
//! Every other invocation in this crate is either read-only or confined to a
//! throwaway worktree. This one is not, and that is the whole point: the user
//! asked for the fixes to be applied to their own checkout. What makes that
//! recoverable is not a sandbox but git — the engine refuses to start unless the
//! working tree is clean, so everything this does shows up in `git status` and
//! nothing of the user's can be buried by it.
//!
//! What *is* confined is where the file tools may point. `Edit` and `Write` are
//! granted as `./**` rules, so the repository is the only tree the CLI will
//! write to without asking, and it has no way to ask. `Bash` cannot be confined
//! that way and is not claimed to be: a shell is a shell. It is granted because
//! the prompt asks for a failing test, a fix, and a commit per defect, and none
//! of that happens without one.
//!
//! No output schema. The answer is a report for a person to read, and forcing it
//! into a shape would cost a turn and tell them less.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::{ProviderError, Run, invoke};

/// Tools applying a fix needs. Writing the change and running the test it leaves
/// behind is the entire task, so unlike a read-only sweep this list allows
/// `Edit`, `Write` and `Bash`.
pub(crate) const APPLY_TOOLS: &str = "Read,Glob,Grep,Edit,Write,Bash";

/// The rules those tools are granted under. `./**` is relative to the working
/// directory, which is the repository — the same scoping a review's reads get,
/// applied to the writes as well.
///
/// `Grep` and `Bash` carry no path: neither takes one as its subject, and a rule
/// that looks like a boundary without being one is worse than an honest bare
/// name.
pub(crate) const APPLY_ALLOWED: &str = "Read(./**),Glob(./**),Grep,Edit(./**),Write(./**),Bash";

/// Denying `WebFetch` and `WebSearch` beside `Bash` is not a network restriction
/// and must not be read as one — `curl` is one command away. It removes the two
/// most convenient paths and costs nothing.
pub(crate) const APPLY_DENIED: &str = "WebFetch,WebSearch";

pub struct ApplyRequest<'a> {
    /// The user's own repository. Written to, deliberately.
    pub repo: &'a Path,
    pub model: &'a str,
    pub effort: &'a str,
    /// The handoff prompt, exactly as it was written to disk.
    pub prompt: &'a str,
    pub timeout: Duration,
    pub max_turns: u32,
    /// Explicit CLI path for tests; real runs use discovery.
    pub binary: Option<&'a str>,
}

/// Run the fixes, returning the model's account of what it did.
///
/// That account is not evidence and is never treated as such: the caller reports
/// it beside the files git says actually changed.
pub async fn apply(request: ApplyRequest<'_>) -> Result<String, ProviderError> {
    let outcome = invoke::<Value>(Run {
        repo: request.repo,
        model: request.model,
        effort: request.effort,
        prompt: request.prompt,
        schema: Value::Null,
        available: APPLY_TOOLS,
        allowed: APPLY_ALLOWED,
        denied: APPLY_DENIED,
        max_turns: request.max_turns,
        timeout: request.timeout,
        binary: request.binary,
        api_key: None,
        session_id: None,
        resume: None,
    })
    .await?;

    Ok(report_text(&outcome.result))
}

/// The reply as text. Without a schema the CLI puts the model's prose in
/// `result` as a JSON string; anything else is shown raw rather than dropped.
fn report_text(result: &Value) -> String {
    match result {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_a_fix_may_write_and_run_commands() {
        for tool in ["Edit", "Write", "Bash"] {
            assert!(
                APPLY_TOOLS.split(',').any(|t| t == tool),
                "applying a fix needs {tool}"
            );
        }
    }

    #[test]
    fn writes_are_granted_only_inside_the_repository() {
        for rule in ["Edit(./**)", "Write(./**)"] {
            assert!(
                APPLY_ALLOWED.split(',').any(|granted| granted == rule),
                "{rule} is not how the write tools are granted: {APPLY_ALLOWED}"
            );
        }
        // A bare `Edit` or `Write` would grant the whole filesystem, which is
        // exactly what the scoped rule exists to prevent.
        for unscoped in ["Edit", "Write", "Read"] {
            assert!(
                !APPLY_ALLOWED.split(',').any(|granted| granted == unscoped),
                "{unscoped} is granted unscoped: {APPLY_ALLOWED}"
            );
        }
    }
}
