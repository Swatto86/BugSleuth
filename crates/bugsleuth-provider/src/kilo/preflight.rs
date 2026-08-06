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

/// The check itself, over an explicit candidate list.
///
/// Split out so the tests can supply a config rather than mutating the
/// environment — a security check whose result depends on the developer's own
/// machine is not a check.
pub(crate) fn gap_in(candidates: &[PathBuf]) -> Option<String> {
    let Some((path, config)) = candidates.iter().find_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
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
