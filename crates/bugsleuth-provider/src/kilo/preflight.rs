//! Whether this machine's Kilo config is safe to sweep with.
//!
//! Claude and Codex are constrained by flags BugSleuth passes itself, so their
//! guarantees hold whatever the machine looks like. Kilo has no such flag: its
//! permissions come from the user's own `kilo.jsonc`, so the guarantee is only
//! as good as that file. This module requires a deny-by-default agent with only
//! the read/search and in-session todo tools left open.
//!
//! **What was measured**, against the real CLI with [`super::BASE_FLAGS`]:
//!
//! - `--auto` does *not* override `deny`. Its help text says it auto-approves
//!   all permissions, but what it actually overrides is `ask`. Under the
//!   globally configured `ask` agent, an edit and a `bash` call were both
//!   refused, and a read outside `--dir` was refused by `external_directory`.
//! - `webfetch` was **not** refused. More importantly, those denials were a
//!   property of one machine's config rather than of the invocation itself.
//!
//! A reviewed repository is attacker input, so checking only network tools is
//! insufficient: shell, external paths, edits, skills, LSPs, subagents and MCP
//! tools can all escape the intended read-only review. Unknown future tools
//! must fail closed too.
//!
//! **Absence is not denial.** `kilo agent list` prints resolved rules but emits
//! no complete permission map, so a scan of its output cannot prove the agent
//! deny-by-default. This reads the config itself; unreadable or incomplete
//! configuration fails closed.

use std::path::PathBuf;

use serde_json::Value;

/// The agent a sweep runs under. Kept next to the check that validates it, so
/// the two cannot drift apart.
pub(crate) const SWEEP_AGENT: &str = "ask";

/// Permissions that cannot mutate the checkout or reach the host/network.
const SAFE_SWEEP_PERMISSIONS: &[&str] = &["read", "glob", "grep", "todoread", "todowrite"];

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

/// Returns why the sweep agent retains a host capability, or `None` when safe.
pub fn permission_gap() -> Option<String> {
    gap_in(&config_paths())
}

/// JSONC with its comments blanked out, so `serde_json` can parse it.
///
/// The file is named `.jsonc` and Kilo accepts comments in it, but
/// `serde_json` rejects them — so a perfectly valid deny-by-default config
/// would be skipped, and the sweep refused with "no readable Kilo
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
            "no readable Kilo config was found, so its permissions cannot be confirmed safe"
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

    let mut open = Vec::<String>::new();
    if !denies(agent, "*") {
        open.push("the default `*` permission".to_string());
    }
    if let Some(rules) = agent.as_object() {
        for tool in rules.keys() {
            if tool != "*"
                && !SAFE_SWEEP_PERMISSIONS.contains(&tool.as_str())
                && !denies(agent, tool)
            {
                open.push(format!("`{tool}`"));
            }
        }
    }

    if open.is_empty() {
        return None;
    }
    Some(format!(
        "the `{SWEEP_AGENT}` agent in {} leaves {} open",
        path.display(),
        open.join(" or ")
    ))
}

fn rule_denies_everything(rule: &Value) -> bool {
    match rule {
        Value::String(action) => action == "deny",
        Value::Object(patterns) => {
            patterns.len() == 1 && patterns.get("*").and_then(Value::as_str) == Some("deny")
        }
        _ => false,
    }
}

/// A rule is a complete denial only when no ordered exception can reopen it.
fn denies(permission: &Value, tool: &str) -> bool {
    if !permission.is_object() {
        return rule_denies_everything(permission);
    }
    let wildcard = permission.get("*");
    if wildcard.is_some_and(|rule| !rule_denies_everything(rule)) {
        return false;
    }
    permission
        .get(tool)
        .or(wildcard)
        .is_some_and(rule_denies_everything)
}

#[cfg(test)]
#[path = "preflight/tests.rs"]
mod tests;
