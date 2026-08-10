use super::*;

fn permission(json: &str) -> Value {
    serde_json::from_str(json).expect("test json")
}

#[test]
fn a_plain_deny_counts() {
    let p = permission(r#"{"webfetch": "deny"}"#);
    assert!(denies(&p, "webfetch"));
}

#[test]
fn a_wildcard_deny_counts() {
    let p = permission(r#"{"webfetch": {"*": "deny"}}"#);
    assert!(denies(&p, "webfetch"));
}

#[test]
fn ask_is_not_denial_because_auto_approves_it() {
    // `--auto` turns every `ask` into a yes, so an `ask` rule is worth
    // nothing to an unattended sweep.
    let p = permission(r#"{"webfetch": "ask"}"#);
    assert!(!denies(&p, "webfetch"));
}

#[test]
fn a_missing_entry_is_not_denial() {
    // Absence must fail closed. A scan of resolved agent output cannot prove
    // whether an omitted entry is denied.
    let p = permission(r#"{"edit": "deny"}"#);
    assert!(!denies(&p, "webfetch"));
}

#[test]
fn a_pattern_list_without_a_wildcard_deny_is_not_denial() {
    // Denying one host leaves every other host open.
    let p = permission(r#"{"webfetch": {"https://evil.test/*": "deny"}}"#);
    assert!(!denies(&p, "webfetch"));
}

#[test]
fn an_allow_exception_makes_a_wildcard_deny_unsafe() {
    let p = permission(r#"{"webfetch":{"*":"deny","https://attacker.example/**":"allow"}}"#);
    assert!(!denies(&p, "webfetch"));
}

#[test]
fn an_open_top_level_wildcard_is_not_hidden_by_a_specific_deny() {
    let p = permission(r#"{"webfetch":"deny","*":"allow"}"#);
    assert!(!denies(&p, "webfetch"));
}

fn config(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-preflight")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("kilo.jsonc");
    std::fs::write(&path, body).expect("write config");
    path
}

#[test]
fn a_commented_config_is_read_rather_than_skipped() {
    // Kilo accepts JSONC; a valid safe config must not be rejected merely for
    // containing comments.
    let path = config(
        "commented",
        "{\n  // the sweep agent\n  \"agent\": {\n    /* block */\n    \"ask\": {\n      \
         \"permission\": {\"*\": \"deny\", \"read\": \"allow\"}\n    }\n  }\n}",
    );
    assert_eq!(gap_in(&[path]), None);
}

#[test]
fn a_url_inside_a_string_is_not_mistaken_for_a_comment() {
    // A plain `//` scan would eat the tail of a URL and make valid JSONC look
    // unreadable.
    let stripped = strip_jsonc(r#"{"baseURL": "http://localhost:11434/api", "x": 1}"#);
    assert!(
        stripped.contains("http://localhost:11434/api"),
        "{stripped}"
    );
    let parsed: Value = serde_json::from_str(&stripped).expect("should still parse");
    assert_eq!(parsed["baseURL"], "http://localhost:11434/api");
    assert_eq!(parsed["x"], 1);
}

#[test]
fn an_escaped_quote_does_not_end_a_string_early() {
    // Without escape tracking, the quote closes the string and the following
    // `//` is misread as a comment.
    let stripped = strip_jsonc(r#"{"a": "say \" then // not a comment", "b": 2}"#);
    let parsed: Value = serde_json::from_str(&stripped).expect("should still parse");
    assert_eq!(parsed["a"], r#"say " then // not a comment"#);
    assert_eq!(parsed["b"], 2);
}

#[test]
fn comments_are_actually_removed() {
    // Guard the guard: a stripper that did nothing would pass the survival
    // tests above.
    let stripped = strip_jsonc("{\n  // gone\n  \"a\": 1 /* also gone */\n}");
    assert!(!stripped.contains("gone"), "{stripped}");
    assert_eq!(
        serde_json::from_str::<Value>(&stripped).expect("parses")["a"],
        1
    );
}

#[test]
fn the_real_machine_config_is_readable_when_present() {
    // Exercise the actual configured path without requiring one to exist on a
    // clean machine. An existing file must reach a verdict, not a parse error.
    let candidates = config_paths();
    if !candidates.iter().any(|p| p.exists()) {
        return;
    }
    if let Some(gap) = gap_in(&candidates) {
        assert!(
            !gap.contains("no readable Kilo config"),
            "a config exists but could not be parsed: {gap}"
        );
    }
}

#[test]
fn a_deny_by_default_config_passes() {
    // A safe configuration must pass; otherwise an always-refusing check looks
    // secure while making Kilo unusable.
    let path = config(
        "closed",
        r#"{"agent":{"ask":{"permission":{"*":"deny","read":"allow","glob":"allow","grep":"allow"}}}}"#,
    );
    assert_eq!(gap_in(&[path]), None);
}

#[test]
fn a_blanket_permission_deny_passes() {
    let path = config("blanket-deny", r#"{"agent":{"ask":{"permission":"deny"}}}"#);
    assert_eq!(gap_in(&[path]), None);
}

#[test]
fn an_allowed_shell_is_a_gap() {
    let path = config(
        "shell-open",
        r#"{"agent":{"ask":{"permission":{"*":"deny","read":"allow","glob":"allow","grep":"allow","bash":"allow"}}}}"#,
    );
    let gap = gap_in(&[path]).expect("an allowed shell must refuse the sweep");
    assert!(gap.contains("bash"), "the gap did not name bash: {gap}");
}

#[test]
fn an_unknown_allowed_tool_is_a_gap() {
    let path = config(
        "unknown-open",
        r#"{"agent":{"ask":{"permission":{"*":"deny","read":"allow","glob":"allow","grep":"allow","mcp_publish":"allow"}}}}"#,
    );
    let gap = gap_in(&[path]).expect("an unknown allowed tool must refuse the sweep");
    assert!(
        gap.contains("mcp_publish"),
        "the gap did not name the unknown tool: {gap}"
    );
}

#[test]
fn an_ask_rule_is_reported_as_a_gap_and_names_the_tool() {
    // `--auto` approves `ask`, so the explicit exception must be named.
    let path = config(
        "open",
        r#"{"agent":{"ask":{"permission":{"*":"deny","webfetch":"ask"}}}}"#,
    );
    let gap = gap_in(&[path]).expect("an `ask` webfetch rule must be a gap");
    assert!(gap.contains("webfetch"), "gap should name the tool: {gap}");
}

#[test]
fn a_missing_config_fails_closed() {
    // No evidence of a safe permission map is a refusal, not a pass.
    let missing = std::env::temp_dir().join("bugsleuth-preflight-does-not-exist.jsonc");
    assert!(gap_in(&[missing]).is_some());
    assert!(gap_in(&[]).is_some(), "no candidates must fail closed");
}

#[test]
fn a_config_without_the_sweep_agent_fails_closed() {
    // Hardening a different agent says nothing about the one the sweep runs.
    let path = config(
        "wrong-agent",
        r#"{"agent":{"plan":{"permission":{"webfetch":"deny"}}}}"#,
    );
    let gap = gap_in(&[path]).expect("hardening another agent is not hardening `ask`");
    assert!(
        gap.contains(SWEEP_AGENT),
        "gap should name the agent: {gap}"
    );
}

#[test]
fn an_apply_agent_that_cannot_edit_is_refused_before_the_run_is_spent() {
    let path = config(
        "apply-denied",
        r#"{"agent":{"code":{"permission":{"edit":"deny"}}}}"#,
    );
    let gap = apply_gap_in(&[path]).expect("an agent that cannot edit must refuse the apply");
    assert!(
        gap.contains(APPLY_AGENT),
        "gap should name the agent: {gap}"
    );
    assert!(
        gap.contains("edit"),
        "gap should name the permission: {gap}"
    );
}

#[test]
fn an_apply_agent_that_can_edit_is_allowed_through() {
    let path = config(
        "apply-allowed",
        r#"{"agent":{"code":{"permission":{"edit":{"*":"allow","**/id_rsa*":"deny"},"task":"ask"}}}}"#,
    );
    assert_eq!(apply_gap_in(&[path]), None);
}

#[test]
fn an_apply_is_not_refused_merely_because_the_machine_has_no_kilo_config() {
    // The apply path makes no confinement claim — Kilo has nothing to make one
    // with — so absence is left to the CLI's own defaults rather than turned
    // into a refusal the user cannot act on. The sweep, which does make such a
    // claim, fails closed on the same input.
    let missing = std::env::temp_dir().join("bugsleuth-preflight-absent/kilo.jsonc");
    assert_eq!(apply_gap_in(&[missing.clone()]), None);
    assert_eq!(apply_gap_in(&[]), None);
    assert!(
        gap_in(&[missing]).is_some(),
        "the sweep must still fail closed on the same input"
    );
}

#[test]
fn hardening_the_sweep_agent_does_not_block_the_apply_agent() {
    // The two live in one file and are read for opposite properties. A config
    // that denies everything to `ask` must still let `code` write.
    let path = config(
        "both-agents",
        r#"{"agent":{"ask":{"permission":{"*":"deny","read":"allow"}},
             "code":{"permission":{"edit":{"*":"allow"}}}}}"#,
    );
    assert_eq!(gap_in(&[path.clone()]), None);
    assert_eq!(apply_gap_in(&[path]), None);
}
