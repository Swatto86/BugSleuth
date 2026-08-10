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
        "--max-turns".into(),
        run.max_turns.to_string(),
        "--tools".into(),
        run.available.into(),
        "--allowedTools".into(),
        run.allowed.into(),
        "--permission-mode".into(),
        "dontAsk".into(),
    ];
    // A null schema means "no schema", not "the schema `null`". Applying fixes
    // is the one invocation whose answer is prose for a person rather than a
    // structure for us, and `--json-schema null` would constrain the reply to
    // the JSON literal `null` — an empty report for work that really happened.
    if !run.schema.is_null() {
        args.push("--json-schema".into());
        args.push(run.schema.to_string());
    }
    // BugSleuth never signs someone else's commits. This CLI adds
    // `Co-Authored-By: Claude …` to every commit it makes unless told not to,
    // and applying fixes asks it to commit one defect at a time — so without
    // this, the tool writes attribution into the user's history as a side
    // effect of being helpful. Measured both ways against a real repository:
    // with the setting the commit is clean, without it the trailer is there.
    //
    // Passed on every invocation, not just the writing one: "this tool does not
    // attribute an AI" is a property of the product, not of one code path, and
    // a sweep that cannot commit is not harmed by a setting it never uses.
    args.push("--settings".into());
    args.push(r#"{"includeCoAuthoredBy":false}"#.into());

    if !run.denied.is_empty() {
        args.push("--disallowedTools".into());
        args.push(run.denied.into());
    }
    if let Some(session) = run.resume {
        args.push("--resume".into());
        args.push(session.to_string());
    } else if let Some(session) = &run.session_id {
        args.push("--session-id".into());
        args.push(session.clone());
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
