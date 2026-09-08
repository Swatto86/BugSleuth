//! Vendor selection and confinement rules for Kimi and Cursor.
//!
//! Split from `tests.rs` at the hard line cap. These assert the closed
//! `Vendor` enum's behaviour for the two adapters that share worktree
//! isolation and no schema enforcement.

use super::*;
/// Kimi is a first-class vendor with Kilo's confinement, not Claude's.
///
/// It has no tool allowlist and no read-only flag — only `--yolo`, which
/// loosens — so the one thing standing between a Kimi review and the code it
/// reviews is that it never sees the real checkout. And it takes no schema, so
/// its brief has to describe the shape in words.
#[test]
fn kimi_is_selected_by_prefix_and_must_be_isolated() {
    assert_eq!(Vendor::parse(" codex:gpt "), (Vendor::Codex, "gpt"));
    assert_eq!(
        Vendor::parse("opencode:kimi-k3"),
        (Vendor::OpenCode, "kimi-k3")
    );
    assert_eq!(Vendor::parse("opencode:"), (Vendor::OpenCode, ""));
    assert_eq!(resolved_label("opencode:kimi-k3"), "opencode:kimi-k3");

    assert!(
        Vendor::OpenCode.needs_isolation(),
        "a Kimi sweep pointed at the real checkout has nothing stopping it writing"
    );
    assert!(
        !Vendor::OpenCode.enforces_schema(),
        "Kimi takes no schema, so its brief must describe the shape instead"
    );
    // The known-present control: Claude differs on both counts, so this is not
    // asserting something true of every vendor.
    assert!(!Vendor::Claude.needs_isolation());
    assert!(Vendor::Claude.enforces_schema());
}

/// A Kimi model in the plan makes Kimi one of the vendors to pre-check.
#[test]
fn a_kimi_model_selects_kimi_for_the_precheck() {
    let models = vec!["sonnet".to_string(), "opencode:kimi-k3".to_string()];
    assert_eq!(
        precheck::vendors_for(&models),
        vec![Vendor::Claude, Vendor::OpenCode]
    );
}

/// Cursor is a first-class vendor: ask-mode is read-only, but sweeps still
/// isolate because there is no ignore-rules flag for project instructions.
#[test]
fn cursor_is_selected_by_prefix_and_must_be_isolated() {
    assert_eq!(
        Vendor::parse("cursor:composer-2.5"),
        (Vendor::Cursor, "composer-2.5")
    );
    assert_eq!(Vendor::parse("cursor:"), (Vendor::Cursor, ""));
    assert_eq!(resolved_label("cursor:composer-2.5"), "cursor:composer-2.5");

    assert!(
        Vendor::Cursor.needs_isolation(),
        "without ignore-rules, a Cursor sweep must not see the real checkout's instructions"
    );
    assert!(
        !Vendor::Cursor.enforces_schema(),
        "Cursor takes no schema, so its brief must describe the shape instead"
    );
    assert!(!Vendor::Claude.needs_isolation());
    assert!(Vendor::Claude.enforces_schema());
}

#[test]
fn a_cursor_model_selects_cursor_for_the_precheck() {
    let models = vec!["sonnet".to_string(), "cursor:composer-2.5".to_string()];
    assert_eq!(
        precheck::vendors_for(&models),
        vec![Vendor::Claude, Vendor::Cursor]
    );
}

#[test]
fn opencode_preserves_local_tags_and_selects_its_own_precheck() {
    let spec = "opencode:ollama/qwen3:8b";
    assert_eq!(Vendor::parse(spec), (Vendor::OpenCode, "ollama/qwen3:8b"));
    assert_eq!(resolved_label(spec), spec);
    assert!(Vendor::OpenCode.needs_isolation());
    assert!(!Vendor::OpenCode.enforces_schema());
    assert_eq!(precheck::vendors_for(&[spec.into()]), [Vendor::OpenCode]);
}

#[test]
fn retired_providers_are_refused_even_without_an_effort() {
    for model in ["kilo:provider/model", "kimi:model", " kilo: "] {
        let error = crate::plan::check_effort(model, "").unwrap_err();
        assert!(error.to_string().contains("support has been removed"));
    }
    assert!(crate::plan::check_effort("opencode:ollama/qwen3:8b", "thinking").is_ok());
    assert_eq!(
        crate::plan::canonical_spec(" opencode: ollama/qwen3:8b "),
        "opencode:ollama/qwen3:8b"
    );
}

#[tokio::test]
async fn retired_provider_sweep_reports_not_swept_without_starting_a_cli() {
    let report = run(Request {
        repo: Path::new("missing-retired-provider-fixture"),
        lane: Lane::Security,
        model: "kilo:old-model",
        scope: None,
        effort: "",
        max_turns: 1,
        timeout: Duration::from_secs(1),
        api_key: None,
        binary: Some("must-not-start"),
    })
    .await;
    assert!(
        matches!(report.status, Status::NotSwept { reason } if reason.contains("support has been removed"))
    );
}
