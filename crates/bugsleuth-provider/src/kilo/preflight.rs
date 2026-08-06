//! Whether this machine's Kilo config is safe to sweep with.
//!
//! Claude and Codex are constrained by flags BugSleuth passes itself, so their
//! guarantees hold whatever the machine looks like. Kilo has no such flag: its
//! permissions come from the user's own `kilo.jsonc`, so the guarantee is only
//! as good as that file. This module checks the one restriction that matters
//! and that Kilo will not enforce on request.
//!
//! **What was measured**, against the real CLI with [`super::BASE_FLAGS`]:
//!
//! - `--auto` does *not* override `deny`. Its help text says it auto-approves
//!   all permissions, but what it actually overrides is `ask`. Under the
//!   globally configured `ask` agent, an edit and a `bash` call were both
//!   refused, and a read outside `--dir` was refused by `external_directory`.
//! - `webfetch` was **not** refused. A sweep holding the reviewed repository's
//!   text — attacker input — could therefore send it to the network.
//!
//! So the network is the live gap, and it is a config setting rather than a
//! missing feature: `webfetch: deny` on the `ask` agent closes it. This check
//! exists so a machine that has not set it fails loudly instead of sweeping.
//!
//! **Absence is not denial.** `kilo agent list` prints resolved rules but emits
//! no `webfetch` entry either way, so a scan of its output cannot tell "denied"
//! from "never mentioned" — and an empty result would read as safe. This reads
//! the config and requires an explicit `deny`, so an unreadable file, a renamed
//! key, or a config that never mentions `webfetch` all fail closed.

use std::path::PathBuf;

use serde_json::Value;

/// The agent a sweep runs under. Kept next to the check that validates it, so
/// the two cannot drift apart.
pub(crate) const SWEEP_AGENT: &str = "ask";

/// Tools that would let a sweep reach the network.
const NETWORK_TOOLS: &[&str] = &["webfetch", "websearch"];

/// Where Kilo reads its configuration from, most specific first.
fn config_paths() -> Vec<PathBuf> {
    // `dirs` is not a dependency and this is the only place that needs a home
    // directory, so the platform variables are read directly. USERPROFILE is
    // the Windows one; HOME covers the rest.
    let Some(home) = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
    else {
        return Vec::new();
    };
    let dir = PathBuf::from(home).join(".config").join("kilo");
    vec![dir.join("kilo.jsonc"), dir.join("kilo.json")]
}

/// Is a network tool explicitly denied for the sweep agent?
///
/// Returns the reason it is *not* safe, or `None` when the config denies every
/// network tool. Phrased that way round so the caller cannot accidentally treat
/// a missing config as a pass.
pub fn network_gap() -> Option<String> {
    gap_in(&config_paths())
}

/// JSONC with its comments blanked out, so `serde_json` can parse it.
///
/// The file is named `.jsonc` and Kilo accepts comments in it, but
/// `serde_json` rejects them — so a perfectly valid config that denies the
/// network would be skipped, and the sweep refused with "no readable Kilo
/// config". Found by BugSleuth reviewing this very function.
///
/// Must be string-aware rather than a plain `//` search: a config with
/// `"http://localhost:11434/api"` in it — as this machine's does — would
/// otherwise have the rest of that line eaten and the whole file fail to parse,
/// turning a working setup into a refusal. Escapes are tracked for the same
/// reason, so a `\"` inside a string does not appear to end it.
///
/// Comments are replaced by spaces rather than removed so byte offsets in any
/// parse error still line up with the original file. Trailing commas are *not*
/// handled: they are rarer than comments, and the failure mode is the safe one.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                // Line comment: blank to the newline, which is kept so line
                // numbers survive.
                out.push(' ');
                out.push(' ');
                chars.next();
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                    out.push(' ');
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                out.push(' ');
                out.push(' ');
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    // Newlines inside a block comment are kept, for the same
                    // reason as above.
                    out.push(if c == '\n' { '\n' } else { ' ' });
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The check itself, over an explicit candidate list.
///
/// Split out so the tests can supply a config rather than mutating the
/// environment — a security check whose result depends on the developer's own
/// machine is not a check.
pub(crate) fn gap_in(candidates: &[PathBuf]) -> Option<String> {
    let Some((path, config)) = candidates.iter().find_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&strip_jsonc(&text)).ok()?;
        Some((path, value))
    }) else {
        return Some(
            "no readable Kilo config was found, so its network access cannot be confirmed denied"
                .to_string(),
        );
    };

    let agent = config
        .get("agent")
        .and_then(|a| a.get(SWEEP_AGENT))
        .and_then(|a| a.get("permission"));

    let Some(agent) = agent else {
        return Some(format!(
            "the `{SWEEP_AGENT}` agent in {} declares no `permission` block",
            path.display()
        ));
    };

    let missing: Vec<&str> = NETWORK_TOOLS
        .iter()
        .copied()
        .filter(|tool| !denies(agent, tool))
        .collect();

    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "the `{SWEEP_AGENT}` agent in {} does not deny {}",
        path.display(),
        missing.join(" or ")
    ))
}

/// A tool is denied when its entry is the string `"deny"`, or an object whose
/// `*` pattern is `deny`. Any other shape — an allow, an `ask` that `--auto`
/// would approve, or a pattern list with holes in it — is not a denial.
fn denies(permission: &Value, tool: &str) -> bool {
    match permission.get(tool) {
        Some(Value::String(action)) => action == "deny",
        Some(Value::Object(patterns)) => patterns.get("*").and_then(Value::as_str) == Some("deny"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
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
        // The whole reason this check exists: `--auto` turns every `ask` into a
        // yes, so an `ask` rule is worth nothing to a sweep.
        let p = permission(r#"{"webfetch": "ask"}"#);
        assert!(!denies(&p, "webfetch"));
    }

    #[test]
    fn a_missing_entry_is_not_denial() {
        // Absence must fail closed. This is the case a scan of `agent list`
        // output would have silently passed.
        let p = permission(r#"{"edit": "deny"}"#);
        assert!(!denies(&p, "webfetch"));
    }

    #[test]
    fn a_pattern_list_without_a_wildcard_deny_is_not_denial() {
        // Denying one host leaves every other host open.
        let p = permission(r#"{"webfetch": {"https://evil.test/*": "deny"}}"#);
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
        // The file is `.jsonc` and Kilo allows comments, so a config that does
        // deny the network must not be reported as unreadable.
        let path = config(
            "commented",
            "{\n  // the sweep agent\n  \"agent\": {\n    /* block */\n    \"ask\": {\n      \
             \"permission\": {\"webfetch\": \"deny\", \"websearch\": \"deny\"}\n    }\n  }\n}",
        );
        assert_eq!(gap_in(&[path]), None);
    }

    #[test]
    fn a_url_inside_a_string_is_not_mistaken_for_a_comment() {
        // This machine's own config contains "http://localhost:11434/api". A
        // naive `//` strip eats the rest of that line, the file stops parsing,
        // and a working setup is refused as unreadable.
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
        // Without escape tracking, the `\"` closes the string, the `//` that
        // follows looks like code rather than text, and the line is eaten.
        let stripped = strip_jsonc(r#"{"a": "say \" then // not a comment", "b": 2}"#);
        let parsed: Value = serde_json::from_str(&stripped).expect("should still parse");
        assert_eq!(parsed["a"], r#"say " then // not a comment"#);
        assert_eq!(parsed["b"], 2);
    }

    #[test]
    fn comments_are_actually_removed() {
        // The guard against the opposite failure: a strip that did nothing
        // would still pass the tests above, since they only check what survives.
        let stripped = strip_jsonc("{\n  // gone\n  \"a\": 1 /* also gone */\n}");
        assert!(!stripped.contains("gone"), "{stripped}");
        assert_eq!(
            serde_json::from_str::<Value>(&stripped).expect("parses")["a"],
            1
        );
    }

    #[test]
    fn the_real_machine_config_is_readable_when_present() {
        // The end-to-end case the unit tests cannot cover: whatever is actually
        // at ~/.config/kilo, this must reach a verdict about the sweep agent
        // rather than failing to parse. A "no readable config" answer here on a
        // machine that has one is the bug this function was written for.
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
    fn a_config_denying_every_network_tool_passes() {
        // Without this half, a check that always refused would look identical
        // to one that works, and Kilo could never sweep at all.
        let path = config(
            "closed",
            r#"{"agent":{"ask":{"permission":{"webfetch":"deny","websearch":"deny"}}}}"#,
        );
        assert_eq!(gap_in(&[path]), None);
    }

    #[test]
    fn an_ask_rule_is_reported_as_a_gap_and_names_the_tool() {
        // The dangerous case: `--auto` approves `ask`, so this reads as
        // configured while leaving the network open.
        let path = config(
            "open",
            r#"{"agent":{"ask":{"permission":{"webfetch":"ask","websearch":"deny"}}}}"#,
        );
        let gap = gap_in(&[path]).expect("an `ask` webfetch rule must be a gap");
        assert!(gap.contains("webfetch"), "gap should name the tool: {gap}");
        assert!(
            !gap.contains("websearch"),
            "websearch is denied and should not be reported: {gap}"
        );
    }

    #[test]
    fn a_missing_config_fails_closed() {
        // The empty-scan case. An unreadable or absent config must refuse, not
        // wave the sweep through for lack of evidence against it.
        let missing = std::env::temp_dir().join("bugsleuth-preflight-does-not-exist.jsonc");
        assert!(gap_in(&[missing]).is_some());
        assert!(gap_in(&[]).is_some(), "no candidates must fail closed");
    }

    #[test]
    fn a_config_without_the_sweep_agent_fails_closed() {
        // A config that hardens some *other* agent says nothing about the one
        // sweeps actually run under.
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
}
