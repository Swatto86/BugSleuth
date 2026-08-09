//! Tests for the Kilo adapter, in their own file only because the module
//! plus its tests crossed the hard line cap.

use super::*;

fn spec<'a>(model: &'a str) -> KiloSweep<'a> {
    KiloSweep {
        effort: "",
        worktree: Path::new("/tmp/wt"),
        model,
        brief: "",
        timeout: Duration::from_secs(60),
        binary: None,
    }
}

/// A scratch directory that is empty at the start of every run.
///
/// Cleared rather than merely created: a process id is reused eventually, and a
/// stale `kilo.jsonc` from an earlier run would be read as this run's fixture —
/// the test would then be asserting against a config it did not write.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-kilo-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}

#[test]
fn the_preflight_checks_the_agent_the_sweep_actually_runs_as() {
    // Not a comparison of two constants — since both sides read
    // `SWEEP_AGENT`, that would agree by construction and could never fail.
    // What must hold is that the preflight's *verdict* is about the agent
    // in the argv: a config hardening only that agent has to pass, and one
    // hardening only a different agent has to fail. Otherwise the check
    // could clear `ask` while the sweep ran as something unverified.
    let args = build_args(&spec("kilo/openai/gpt-5.6-sol"));
    let agent = args
        .iter()
        .position(|a| a == "--agent")
        .and_then(|i| args.get(i + 1))
        .expect("a sweep must name an agent")
        .clone();

    let denied = r#"{"*":"deny","read":"allow","glob":"allow","grep":"allow"}"#;
    let dir = scratch("agent-agreement");

    let matching = dir.join("kilo.jsonc");
    std::fs::write(
        &matching,
        format!(r#"{{"agent":{{"{agent}":{{"permission":{denied}}}}}}}"#),
    )
    .expect("write config");
    assert_eq!(
        preflight::gap_in(&[matching]),
        None,
        "hardening the agent the sweep runs as should satisfy the preflight"
    );

    // The same config with only the agent name changed. `is_some()` alone would
    // also pass for an unwritten, unreadable, or unparseable file — every one of
    // which is the wrong reason — so the gap has to name the agent it looked for
    // and say it was the permission block that was missing.
    let other = dir.join("other.jsonc");
    std::fs::write(
        &other,
        format!(r#"{{"agent":{{"not-{agent}":{{"permission":{denied}}}}}}}"#),
    )
    .expect("write config");
    let gap = preflight::gap_in(&[other])
        .expect("hardening a different agent must not satisfy the preflight");
    assert!(
        gap.contains(&agent) && gap.contains("permission"),
        "the refusal must be about the sweep's own agent, not an unreadable file: {gap}"
    );
}

#[test]
fn the_model_id_says_which_account_a_sweep_will_spend_from() {
    // The same model reached three ways bills to three different places.
    assert_eq!(route_of("kilo/z-ai/glm-5"), Route::Gateway);
    assert_eq!(route_of("openrouter/z-ai/glm-5"), Route::OpenRouter);
    assert_eq!(route_of("openai/gpt-5"), Route::OpenAi);
    assert_eq!(route_of("ollama/llama3"), Route::Ollama);
}

#[test]
fn an_id_with_no_prefix_leaves_the_choice_to_kilo() {
    assert_eq!(route_of(""), Route::Configured);
    assert_eq!(route_of("  "), Route::Configured);
    assert_eq!(route_of("some-model"), Route::Configured);
}

#[test]
fn every_route_can_be_described_to_a_person() {
    for route in [
        Route::Gateway,
        Route::OpenRouter,
        Route::OpenAi,
        Route::Ollama,
        Route::Configured,
    ] {
        assert!(!route.describe().is_empty());
    }
}

#[test]
fn external_plugins_are_skipped_so_the_machine_cannot_change_the_review() {
    assert!(build_args(&spec("")).iter().any(|a| a == "--pure"));
}

#[test]
fn the_working_directory_is_pinned_explicitly() {
    let args = build_args(&spec(""));
    let dir = args
        .iter()
        .position(|a| a == "--dir")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    assert_eq!(dir, Some("/tmp/wt"));
}

#[test]
fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
    assert!(!build_args(&spec("   ")).iter().any(|a| a == "-m"));
}
