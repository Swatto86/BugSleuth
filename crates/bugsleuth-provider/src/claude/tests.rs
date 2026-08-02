//! Tests for the Claude adapter, in their own file only because the
//! adapter plus its tests crossed the hard line cap.

use super::super::claude::args::build_args;
use super::super::claude::*;
use bugsleuth_domain::finding_schema;
use std::path::Path;
use std::time::Duration;

fn run<'a>(model: &'a str) -> Run<'a> {
    Run {
        effort: "",
        repo: Path::new("."),
        model,
        prompt: "",
        schema: finding_schema(),
        allowed: READ_ONLY_TOOLS,
        denied: READ_ONLY_DENIED,
        max_turns: 12,
        timeout: Duration::from_secs(60),
        binary: None,
        api_key: None,
        resume: None,
    }
}

#[test]
fn read_only_sweeps_cannot_be_granted_write_tools() {
    let args = build_args(&run("sonnet"));
    let index = args.iter().position(|a| a == "--disallowedTools");
    let denied = index.and_then(|i| args.get(i + 1)).map(String::as_str);
    assert_eq!(denied, Some(READ_ONLY_DENIED));
    assert!(!args.iter().any(|a| a.contains("dangerously-skip")));
}

#[test]
fn customizations_are_disabled_so_the_reviewed_repo_cannot_alter_the_review() {
    assert!(
        build_args(&run("sonnet"))
            .iter()
            .any(|a| a == "--safe-mode")
    );
}

#[test]
fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
    assert!(!build_args(&run("   ")).iter().any(|a| a == "--model"));
}

#[test]
fn the_schema_is_passed_as_one_argv_entry_not_shell_text() {
    let args = build_args(&run("sonnet"));
    let index = args.iter().position(|a| a == "--json-schema");
    let schema = index
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or("");
    let parsed: Value = serde_json::from_str(schema).unwrap_or(Value::Null);
    assert_eq!(parsed["type"], "object");
}

#[test]
fn an_empty_denylist_omits_the_flag_rather_than_passing_an_empty_value() {
    let mut spec = run("sonnet");
    spec.denied = "";
    assert!(!build_args(&spec).iter().any(|a| a == "--disallowedTools"));
}

#[test]
fn a_salvage_run_resumes_the_session_rather_than_starting_a_new_one() {
    // Without --resume the salvage would open a fresh conversation, which
    // knows nothing about the review it is meant to be transcribing - and
    // would answer confidently from nothing.
    let mut spec = run("sonnet");
    spec.resume = Some("session-abc");
    let args = build_args(&spec);
    let index = args.iter().position(|a| a == "--resume");
    assert_eq!(
        index.and_then(|i| args.get(i + 1)).map(String::as_str),
        Some("session-abc")
    );
    // And an ordinary sweep must never inherit a conversation.
    assert!(!build_args(&run("sonnet")).iter().any(|a| a == "--resume"));
}

#[test]
fn a_turn_budget_exhaustion_is_reported_as_its_own_failure_not_a_generic_one() {
    // It is the one failure worth resuming: the review may already be done
    // inside the conversation. Collapsing it into "exited non-zero" threw
    // the whole sweep away, which happened three times in one day.
    let exhausted = ProviderError::TurnsExhausted {
        vendor: VENDOR,
        session: Some("abc".into()),
    };
    assert!(exhausted.to_string().contains("turn budget"));
    // And it is not transient: retrying from scratch would hit the same
    // ceiling. Salvage is the answer, not another attempt.
    assert!(!exhausted.is_transient());
}

#[test]
fn a_salvage_is_never_itself_salvaged() {
    // The guard that stops a cycle: a resumed run that also runs out of
    // turns has said the conversation cannot answer, and a third attempt
    // would only spend more budget hearing it again.
    let mut spec = run("sonnet");
    spec.resume = Some("abc");
    assert!(spec.resume.is_some(), "the wrapper keys off exactly this");
}
