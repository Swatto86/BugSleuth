//! Tests for the Codex adapter, in their own file only because the module
//! plus its tests crossed the hard line cap.

use super::*;

fn spec<'a>(model: &'a str, sandbox: Sandbox) -> Invoke<'a> {
    Invoke {
        effort: "",
        dir: Path::new("."),
        model,
        brief: "",
        timeout: Duration::from_secs(60),
        binary: None,
        schema: finding_schema(),
        sandbox,
    }
}

fn args_for(model: &str, sandbox: Sandbox) -> Vec<String> {
    build_args(
        &spec(model, sandbox),
        Path::new("s.json"),
        Path::new("a.json"),
    )
}

#[test]
fn a_sweep_runs_read_only_and_ignores_the_reviewed_repos_own_config() {
    let joined = args_for("gpt-5.6-codex", Sandbox::ReadOnly).join(" ");
    assert!(joined.contains("--sandbox read-only"));
    assert!(joined.contains("--ignore-user-config"));
    assert!(joined.contains("--ignore-rules"));
    assert!(!joined.contains("dangerously"));
}

#[test]
fn a_proof_attempt_may_write_because_it_has_to_add_a_test() {
    let joined = args_for("", Sandbox::WorkspaceWrite).join(" ");
    assert!(joined.contains("--sandbox workspace-write"));
    // Still never the escape hatch, even when writing is allowed.
    assert!(!joined.contains("dangerously"));
}

#[test]
fn the_prompt_comes_from_stdin_so_a_long_brief_cannot_overflow_the_command_line() {
    let args = args_for("", Sandbox::ReadOnly);
    assert_eq!(args.last().map(String::as_str), Some("-"));
}

#[test]
fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
    assert!(!args_for("  ", Sandbox::ReadOnly).iter().any(|a| a == "-m"));
}

#[test]
fn the_schema_and_answer_paths_are_passed_as_files_not_inline_json() {
    let args = args_for("", Sandbox::ReadOnly);
    let after = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    assert_eq!(after("--output-schema"), Some("s.json"));
    assert_eq!(after("--output-last-message"), Some("a.json"));
}

/// A sign-in check must invoke the CLI the way a sweep does.
///
/// Otherwise it is not checking the sweep: it can pass while every real run
/// fails, or — what actually happened — fail while runs are fine. The first
/// version dropped `--skip-git-repo-check`, so Codex refused with "Not
/// inside a trusted directory" and a working session was reported as
/// unusable, pointing the reader at the wrong problem entirely.
#[test]
fn the_signin_probe_and_a_sweep_share_one_flag_list() {
    let source = include_str!("../codex.rs");
    let code = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(before, _)| before);
    assert_eq!(
        code.matches("SHARED_FLAGS").count(),
        3,
        "the sign-in probe and build_args must both use SHARED_FLAGS, or they \
             can drift and the check stops testing what a run does"
    );
}

/// The two flags that keep a review reproducible and unsteerable.
///
/// Without them the invocation picks up the machine's own Codex
/// configuration and the reviewed repository's rules — and the reviewed
/// repository is untrusted input whose text can address the model directly.
#[test]
fn the_configuration_and_rules_of_the_host_and_the_repo_are_both_ignored() {
    assert!(SHARED_FLAGS.contains(&"--ignore-user-config"));
    assert!(SHARED_FLAGS.contains(&"--ignore-rules"));
    assert!(SHARED_FLAGS.contains(&"--skip-git-repo-check"));
}
