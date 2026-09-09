//! Tests for the model catalogue, split from `models.rs` at the hard line cap.

use super::*;

#[test]
fn claude_versions_are_distinct_choices_with_efforts_and_existing_aliases() {
    let catalogue = claude_models();
    for (label, id, alias) in [
        ("Fable 5.1", "claude-fable-5-1", "fable"),
        ("Fable 5", "claude-fable-5", "fable"),
        ("Opus 5", "claude-opus-5", "opus"),
        ("Sonnet 5", "claude-sonnet-5", "sonnet"),
        ("Haiku 4.5", "claude-haiku-4-5-20251001", "haiku"),
    ] {
        assert!(
            catalogue
                .groups
                .iter()
                .any(|g| g.label == label && g.models.contains(&id.to_string()))
        );
        assert_eq!(
            catalogue.efforts_by_model.get(id),
            catalogue.efforts_by_model.get(alias)
        );
        assert!(catalogue.efforts_by_model.contains_key(id));
    }
}

#[test]
fn a_failed_codex_listing_uses_the_fallback() {
    let live = r#"{"models":[{"slug":"live-only","visibility":"list","supported_reasoning_levels":[{"effort":"high"}]}]}"#;
    let output = |code| process::CliOutput {
        code: Some(code),
        stdout: live.to_string(),
        stderr: String::new(),
    };
    let ids = |catalogue: VendorCatalogue| {
        catalogue
            .groups
            .into_iter()
            .flat_map(|group| group.models)
            .collect::<Vec<_>>()
    };

    let failed = ids(codex_catalogue_from_output(output(1)));
    assert!(
        failed.iter().any(|model| model == "gpt-5.6-codex"),
        "the known fallback vanished: {failed:?}"
    );
    assert!(
        !failed.iter().any(|model| model == "live-only"),
        "failed-process stdout was trusted: {failed:?}"
    );

    let succeeded = ids(codex_catalogue_from_output(output(0)));
    assert!(
        succeeded.iter().any(|model| model == "live-only"),
        "a successful live catalogue was ignored: {succeeded:?}"
    );
    assert!(
        !succeeded.iter().any(|model| model == "gpt-5.6-codex"),
        "the live catalogue was replaced by the fallback: {succeeded:?}"
    );
}

#[test]
fn effort_not_supported_by_codex_model_is_rejected() {
    // Codex's vendor-wide list is empty on purpose: accepted levels are a
    // property of the model. `gpt-5.5` takes up to `xhigh` and rejects `max`.
    let mut catalogue = VendorCatalogue::default();
    catalogue.efforts_by_model.insert(
        "gpt-5.5".to_string(),
        vec![
            "low".to_string(),
            "medium".to_string(),
            "high".to_string(),
            "xhigh".to_string(),
        ],
    );

    assert!(effort_ok("codex", &catalogue, "gpt-5.5", "xhigh").is_ok());
    assert!(
        effort_ok("codex", &catalogue, "gpt-5.5", "max").is_err(),
        "`max` is not in the model's catalogue and must be refused"
    );
    // A model absent from the catalogue cannot be verified, so it is refused
    // rather than forwarded unchecked.
    assert!(effort_ok("codex", &catalogue, "gpt-absent", "high").is_err());
    // Empty effort is always fine — the CLI's own default.
    assert!(effort_ok("codex", &catalogue, "gpt-5.5", "").is_ok());
    assert!(effort_ok("codex", &catalogue, "gpt-5.5", "   ").is_ok());
    // Other vendors are not gated here; their efforts are checked elsewhere.
    assert!(effort_ok("claude", &catalogue, "opus", "max").is_ok());
}

#[test]
fn claude_effort_is_recorded_per_model() {
    // The fixed catalogue, not `available()`: that refuses when the CLI is
    // missing, and CI runners do not install Claude Code.
    assert!(efforts("claude").is_empty());
    let catalogue = claude_models();
    assert_eq!(
        catalogue.efforts_by_model.get("opus"),
        Some(
            &vec!["low", "medium", "high", "xhigh", "max"]
                .into_iter()
                .map(String::from)
                .collect()
        )
    );
    assert_eq!(
        catalogue.efforts_by_model.get("sonnet"),
        Some(
            &vec!["low", "medium", "high", "xhigh", "max"]
                .into_iter()
                .map(String::from)
                .collect()
        )
    );
    assert!(
        catalogue
            .efforts_by_model
            .get("haiku")
            .is_some_and(Vec::is_empty)
    );
}

#[tokio::test]
async fn claude_effort_is_rejected_when_the_model_cannot_apply_it() {
    assert!(
        validate_effort("claude", "fable", "ultracode")
            .await
            .is_ok()
    );
    assert!(validate_effort("claude", "opus", "xhigh").await.is_ok());
    assert!(validate_effort("claude", "sonnet", "xhigh").await.is_ok());
    assert!(validate_effort("claude", "haiku", "high").await.is_err());
}

#[test]
fn cli_installed_rejects_unknown_vendors() {
    assert!(!cli_installed("no-such-vendor"));
    assert!(!cli_installed(""));
}

#[tokio::test]
async fn available_checks_install_before_inventing_a_catalogue() {
    // Unknown names are the always-missing case every machine can exercise —
    // Claude and Codex used to return fixed aliases with no install check.
    assert!(!cli_installed("no-such-vendor"));
    assert!(
        matches!(
            available("no-such-vendor").await,
            Err(ProviderError::NotFound { .. })
        ),
        "an unknown vendor must not invent a catalogue"
    );
    for vendor in VENDORS {
        if cli_installed(vendor) {
            continue;
        }
        assert!(
            available(vendor).await.is_err(),
            "{vendor} offered models without a CLI on PATH"
        );
    }
}

#[test]
fn available_consults_cli_installed_before_any_catalogue() {
    // A check that only runs when a CLI is missing is vacuous on a fully
    // equipped machine. The production gate must still be present in source.
    let source = include_str!("../models.rs");
    let fn_start = source
        .find("pub async fn available")
        .expect("available() is gone");
    let body = &source[fn_start..];
    let match_arm = body
        .find("match vendor")
        .expect("available() no longer dispatches by vendor");
    let gate = body[..match_arm]
        .find("cli_installed(vendor)")
        .expect("available() no longer refuses a missing CLI before building a catalogue");
    assert!(gate < match_arm);
}

#[test]
fn claude_catalogue_offers_the_documented_fable_alias() {
    // Fixed catalogue — same reason as `claude_effort_is_recorded_per_model`.
    let catalogue = claude_models();
    let models: Vec<&str> = catalogue
        .groups
        .iter()
        .flat_map(|group| group.models.iter().map(String::as_str))
        .collect();
    assert!(
        models.contains(&"sonnet"),
        "catalogue scan found no known alias"
    );
    assert!(
        models.contains(&"fable"),
        "the CLI documents the fable alias"
    );
    assert_eq!(
        catalogue.efforts_by_model.get("fable"),
        Some(
            &vec!["low", "medium", "high", "xhigh", "max"]
                .into_iter()
                .map(String::from)
                .collect()
        )
    );
}
