//! Turning one `Run` into the CLI's argument list.
//!
//! Split out because the flags are the contract with the CLI: which tools a
//! review may touch, how its output is constrained, and whether it starts a
//! conversation or continues one. Each is a decision with a reason, and they
//! were getting lost among the code that merely calls the process.

use super::Run;

pub(super) fn build_args(run: &Run<'_>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--print".into(),
        "--safe-mode".into(),
        "--output-format".into(),
        "json".into(),
        "--json-schema".into(),
        run.schema.to_string(),
        "--max-turns".into(),
        run.max_turns.to_string(),
        "--allowedTools".into(),
        run.allowed.into(),
    ];
    if !run.denied.is_empty() {
        args.push("--disallowedTools".into());
        args.push(run.denied.into());
    }
    if let Some(session) = run.resume {
        args.push("--resume".into());
        args.push(session.to_string());
    }
    if !run.model.trim().is_empty() {
        args.push("--model".into());
        args.push(run.model.trim().to_string());
    }
    if !run.effort.trim().is_empty() {
        args.push("--effort".into());
        args.push(run.effort.trim().to_string());
    }
    args
}
