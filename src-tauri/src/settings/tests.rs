//! Tests for what the app remembers between launches.
//!
//! In their own file only because the module plus its tests crossed the hard
//! line cap. They exercise `super`'s real load and save against scratch files
//! rather than a stand-in, because the defects worth catching here — a
//! migration that drops a field, a save that truncates — only exist on disk.

use super::*;

#[test]
fn the_default_covers_every_lane_so_a_first_run_has_no_silent_holes() {
    let settings = Settings::default();
    let covered: Vec<&str> = settings
        .models
        .iter()
        .flat_map(|m| m.lanes.iter().map(String::as_str))
        .collect();
    for lane in ["correctness", "security", "contract", "ux", "gate"] {
        assert!(covered.contains(&lane), "{lane} has no model by default");
    }
}

#[test]
fn unknown_fields_and_missing_fields_both_survive_a_round_trip() {
    let newer = r#"{
        "repo":"C:/x",
        "theme":"dark",
        "future_setting":{"enabled":true},
        "models":[{
            "id":"sonnet",
            "lanes":["contract"],
            "future_model_setting":7
        }]
    }"#;
    let parsed: Settings = serde_json::from_str(newer).expect("newer settings load");
    assert_eq!(parsed.models[0].passes, 1, "missing fields still default");
    assert!(!parsed.models[0].use_agents, "agents must be opt-in");

    let saved = serde_json::to_value(parsed).expect("settings serialize");
    assert_eq!(saved["future_setting"]["enabled"], true);
    assert_eq!(saved["models"][0]["future_model_setting"], 7);
}

#[test]
fn known_lane_spellings_are_canonical_on_the_wire() {
    let parsed: Settings = serde_json::from_str(
        r#"{"models":[{"id":"sonnet","lanes":[" Correctness ","SECURITY","future"]}]}"#,
    )
    .expect("settings load");
    assert_eq!(
        parsed.models[0].lanes,
        ["correctness", "security", "future"]
    );

    let saved = serde_json::to_value(parsed).expect("settings serialize");
    assert_eq!(
        saved["models"][0]["lanes"],
        serde_json::json!(["correctness", "security", "future"])
    );
}

#[test]
fn theme_values_are_valid_before_crossing_to_the_window() {
    for (raw, expected) in [
        ("system", "system"),
        ("light", "light"),
        ("dark", "dark"),
        ("contrast", "system"),
    ] {
        let parsed: Settings =
            serde_json::from_str(&format!(r#"{{"theme":"{raw}"}}"#)).expect("settings load");
        let saved = serde_json::to_value(parsed).expect("settings serialize");
        assert_eq!(saved["theme"], expected, "theme migration for {raw}");
    }
}

#[test]
fn retired_claude_sessions_is_removed_when_settings_load() {
    // The Claude session count was a number the user had to guess at and is
    // now sized from each run's plan. A file that still carries it must open,
    // and must not carry it forward as if the control still existed.
    let dir = scratch("retired-claude-sessions");
    let path = dir.join("settings.json");
    std::fs::write(&path, r#"{"claude_sessions":6,"repo":"C:/x"}"#).expect("write settings");

    let loaded = load_from(&path).expect("load settings");
    assert_eq!(loaded.repo, "C:/x");
    let saved = serde_json::to_value(loaded).expect("serialize settings");
    assert!(saved.get("claude_sessions").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn retired_provider_concurrency_is_removed_when_settings_load() {
    let dir = scratch("retired-provider-concurrency");
    let path = dir.join("settings.json");
    std::fs::write(&path, r#"{"provider_concurrency":10}"#).expect("write settings");

    let loaded = load_from(&path).expect("load settings");
    let saved = serde_json::to_value(loaded).expect("serialize settings");
    assert!(saved.get("provider_concurrency").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

// The "a failed save leaves the settings worse than they were" test used to
// live here and exercised `std::fs::rename` directly rather than anything in
// this file, so it would have passed with `save` still truncating in place.
// `save` now goes through `bugsleuth_engine::atomic::write`, and that module
// tests the real behaviour: a write that fails leaves the previous file
// intact and no debris beside it.

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-settings-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn malformed_settings_are_reported_and_left_in_place() {
    // The defect: a corrupt file was silently replaced with defaults, which
    // the frontend then persisted over it — the configuration gone with no
    // warning. Loading must error instead, and must not touch the file.
    let dir = scratch("malformed");
    let path = dir.join("settings.json");
    let original = "{ this is not json";
    std::fs::write(&path, original).expect("write");

    assert!(
        load_from(&path).is_err(),
        "a corrupt settings file was accepted as defaults"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        original,
        "the corrupt file was overwritten by a mere load"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_read_error_that_is_not_a_missing_file_is_surfaced() {
    // A path that is a directory is readable-but-not-a-file: the read fails
    // with something other than NotFound, and that must propagate rather
    // than be read as first-launch.
    let dir = scratch("read-error");
    assert!(
        load_from(&dir).is_err(),
        "a non-NotFound read error was swallowed as defaults"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_settings_file_is_first_launch_and_yields_defaults() {
    let dir = scratch("missing");
    let settings = load_from(&dir.join("settings.json")).expect("missing file is not an error");
    assert!(!settings.models.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
