//! Handing the fix prompt to Claude, with write access to the real repository.
//!
//! Every other invocation in this crate is either read-only or confined to a
//! throwaway worktree. This one is not, and that is the whole point: the user
//! asked for the fixes to be applied to their own checkout. What makes that
//! recoverable is not a sandbox but git — the engine refuses to start unless the
//! working tree is clean, so everything this does shows up in `git status` and
//! nothing of the user's can be buried by it.
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
///
/// Denying `WebFetch` and `WebSearch` beside `Bash` is not a network restriction
/// and must not be read as one — `curl` is one command away. It removes the two
/// most convenient paths and costs nothing.
const APPLY_TOOLS: &str = "Read,Glob,Grep,Edit,Write,Bash";
const APPLY_DENIED: &str = "WebFetch,WebSearch";

pub struct ApplyRequest<'a> {
    /// The user's own repository. Written to, deliberately.
    pub repo: &'a Path,
    pub model: &'a str,
    pub effort: &'a str,
    /// The handoff prompt, exactly as it was written to disk.
    pub prompt: &'a str,
    pub timeout: Duration,
    pub max_turns: u32,
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
        allowed: APPLY_TOOLS,
        denied: APPLY_DENIED,
        max_turns: request.max_turns,
        timeout: request.timeout,
        binary: None,
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
        assert!(APPLY_DENIED.split(',').any(|t| t == "WebFetch"));
    }

    #[test]
    fn a_prose_reply_survives_having_no_schema_to_fit() {
        assert_eq!(report_text(&Value::String("fixed 3".into())), "fixed 3");
        // An empty answer reads as an empty report, not as the word "null".
        assert_eq!(report_text(&Value::Null), "");
        assert!(report_text(&serde_json::json!({"done": true})).contains("done"));
    }
}
