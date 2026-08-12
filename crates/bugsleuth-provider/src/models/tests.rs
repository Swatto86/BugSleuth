//! Tests for the model catalogue, split from `models.rs` at the hard line cap.

use super::*;

/// Abridged from real `kilo models --verbose` output, shape preserved.
const VERBOSE: &str = r#"
kilo/ai21/jamba-large-1.7
{
  "id": "ai21/jamba-large-1.7",
  "variants": {},
  "hasUserByokAvailable": false
}
kilo/zai-coding/glm-5.2
{
  "id": "zai-coding/glm-5.2",
  "variants": { "high": {}, "max": {} },
  "hasUserByokAvailable": true
}
kilo/z-ai/glm-5.2
{
  "id": "z-ai/glm-5.2",
  "variants": { "low": {}, "medium": {}, "high": {} },
  "hasUserByokAvailable": false
}
openrouter/anthropic/claude-opus-5
{
  "id": "anthropic/claude-opus-5"
}
ollama/qwen3-coder
{
  "id": "qwen3-coder"
}
"#;

#[test]
fn a_kilo_model_billed_to_your_own_plan_is_not_filed_under_the_subscription() {
    // `kilo/z-ai/glm-5.2` spends Kilo Gateway credit and
    // `kilo/zai-coding/glm-5.2` spends a plan you bought from Z.ai. Same
    // model, same prefix, different bill — and the id cannot tell you.
    let catalogue = group_by_route(VERBOSE);
    let group_of = |id: &str| {
        catalogue
            .groups
            .iter()
            .find(|g| g.models.iter().any(|m| m == id))
            .unwrap_or_else(|| panic!("{id} is in no group"))
            .label
            .as_str()
    };
    assert!(group_of("kilo/zai-coding/glm-5.2").contains("BYOK"));
    assert!(group_of("kilo/z-ai/glm-5.2").contains("Gateway"));
    assert_ne!(
        group_of("kilo/zai-coding/glm-5.2"),
        group_of("kilo/z-ai/glm-5.2")
    );
}

#[test]
fn a_failed_kilo_listing_is_an_error_not_an_empty_catalogue() {
    // A non-zero exit with no output cannot be told apart from an empty list
    // unless the status is read — so it must be an error, not Ok(empty).
    let silent = kilo_catalogue_from_output(process::CliOutput {
        code: Some(1),
        stdout: String::new(),
        stderr: String::new(),
    });
    assert!(
        matches!(silent, Err(ProviderError::FailedSilently { .. })),
        "{silent:?}"
    );

    // A non-zero exit that said why keeps the diagnostic.
    let loud = kilo_catalogue_from_output(process::CliOutput {
        code: Some(1),
        stdout: String::new(),
        stderr: "not signed in to Kilo".into(),
    });
    let Err(ProviderError::Failed { message, .. }) = loud else {
        panic!("a diagnostic failure lost its message: {loud:?}");
    };
    assert!(message.contains("not signed in"), "{message}");

    // A clean exit that parsed to nothing is still not a real catalogue.
    let empty = kilo_catalogue_from_output(process::CliOutput {
        code: Some(0),
        stdout: "some banner, no models\n".into(),
        stderr: String::new(),
    });
    assert!(
        matches!(empty, Err(ProviderError::Envelope { .. })),
        "{empty:?}"
    );

    // A clean exit with a real listing succeeds and carries a known model.
    let ok = kilo_catalogue_from_output(process::CliOutput {
        code: Some(0),
        stdout: VERBOSE.to_string(),
        stderr: String::new(),
    })
    .unwrap_or_else(|e| panic!("a valid listing was rejected: {e:?}"));
    let ids: Vec<String> = ok.groups.iter().flat_map(|g| g.models.clone()).collect();
    assert!(
        ids.iter().any(|m| m == "kilo/z-ai/glm-5.2"),
        "the parsed catalogue is missing a known model: {ids:?}"
    );
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
fn efforts_are_recorded_per_model_and_only_where_the_model_has_any() {
    let catalogue = group_by_route(VERBOSE);
    assert_eq!(
        catalogue.efforts_by_model.get("kilo/zai-coding/glm-5.2"),
        Some(&vec!["high".to_string(), "max".to_string()])
    );
    assert_eq!(
        catalogue.efforts_by_model.get("kilo/z-ai/glm-5.2"),
        Some(&vec![
            "low".to_string(),
            "medium".to_string(),
            "high".to_string()
        ]),
        "the same model on two routes can still accept different efforts"
    );
    // Absent rather than present-and-empty: the UI keys off absence to
    // disable the control, and an empty vec would read as "loading".
    assert!(
        !catalogue
            .efforts_by_model
            .contains_key("kilo/ai21/jamba-large-1.7")
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
