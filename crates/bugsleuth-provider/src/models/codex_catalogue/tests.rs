//! Abridged from real `codex debug models` output. The shape matters, not the
//! volume: the live response is a few hundred kilobytes, almost all of it each
//! model's system prompt, and none of that is read.

use super::*;

const SAMPLE: &str = r#"{"models":[
  {"slug":"gpt-5.6-sol","display_name":"GPT-5.6-Sol","visibility":"list",
   "supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"},{"effort":"high"},
                                 {"effort":"xhigh"},{"effort":"max"},{"effort":"ultra"}],
   "base_instructions":"You are Codex, an agent based on GPT-5."},
  {"slug":"gpt-5.5","display_name":"GPT-5.5","visibility":"list",
   "supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"},{"effort":"high"},{"effort":"xhigh"}]},
  {"slug":"codex-auto-review","display_name":"Auto Review","visibility":"hide",
   "supported_reasoning_levels":[{"effort":"low"}]}
]}"#;

#[test]
fn a_hidden_model_is_not_offered() {
    let entries = parse(SAMPLE).expect("the sample is a catalogue");

    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["gpt-5.6-sol", "gpt-5.5"]);
    assert!(
        !ids.contains(&"codex-auto-review"),
        "a model the CLI hides was put in the menu"
    );
}

#[test]
fn effort_levels_are_per_model_and_not_all_the_same() {
    // The whole reason this file exists. A single vendor-wide list offered
    // `max` on models that reject it and never offered `ultra` at all.
    let entries = parse(SAMPLE).expect("the sample is a catalogue");
    let map = efforts(&entries);

    assert_eq!(
        map.get("gpt-5.6-sol").map(Vec::as_slice),
        Some(
            ["low", "medium", "high", "xhigh", "max", "ultra"]
                .map(String::from)
                .as_slice()
        )
    );
    assert_eq!(
        map.get("gpt-5.5").map(Vec::as_slice),
        Some(
            ["low", "medium", "high", "xhigh"]
                .map(String::from)
                .as_slice()
        )
    );
    assert_ne!(
        map.get("gpt-5.6-sol"),
        map.get("gpt-5.5"),
        "if these were equal a vendor-wide list would have been adequate"
    );
}

#[test]
fn output_that_is_not_a_catalogue_is_refused_rather_than_read_as_empty() {
    // The caller falls back to a known list on None. Returning an empty Vec
    // instead would replace a working menu with nothing, which reads as a
    // vendor that has no models rather than as a command that failed.
    assert!(parse("").is_none());
    assert!(parse("error: unknown subcommand `debug`").is_none());
    assert!(
        parse(r#"{"models":[]}"#).is_none(),
        "an empty catalogue is not an answer"
    );
    assert!(
        parse(r#"{"models":[{"slug":"x","visibility":"hide"}]}"#).is_none(),
        "a catalogue with nothing visible is not an answer either"
    );
}

#[test]
fn leading_output_before_the_json_does_not_defeat_it() {
    let noisy = format!("reading catalogue...\n{SAMPLE}");
    assert!(parse(&noisy).is_some(), "a progress line stopped the parse");
}
