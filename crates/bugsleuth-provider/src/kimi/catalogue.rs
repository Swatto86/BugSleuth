//! Which Kimi models this machine can actually pick.
//!
//! Kimi has no `models` list command, so the menu is read from the same file
//! the CLI itself reads: `~/.kimi-code/config.toml`. That is the only honest
//! source. Which aliases exist depends on the account — a subscription and a
//! bring-your-own-key setup reach different sets — so a list hard-coded here
//! would offer models that do not exist for this user, which is worse than
//! offering none.
//!
//! Read with a line scanner rather than a TOML parser, and deliberately so:
//! adding a TOML dependency to read two keys is not worth it, and the scan is
//! written to *under*-report rather than guess. A section it cannot parse is
//! skipped, and the model box still accepts a typed id — so the worst case is
//! the menu this feature replaced.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::models::{ModelGroup, VendorCatalogue};

/// Where the CLI keeps its configuration under a given home directory.
fn config_under(home: PathBuf) -> PathBuf {
    home.join(".kimi-code/config.toml")
}

/// Where the CLI keeps its configuration.
fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(config_under(PathBuf::from(home)))
}

/// The models the local Kimi configuration defines.
///
/// Empty when there is no readable config, which the desktop reports with a
/// reason rather than as a silently empty dropdown.
pub(crate) fn catalogue() -> VendorCatalogue {
    let text = config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    VendorCatalogue {
        groups: groups_in(&text),
        efforts_by_model: BTreeMap::new(),
    }
}

/// One group per model, labelled with its display name.
///
/// The picker shows a group's label beside each of its models, and for Kimi the
/// useful thing to show is `K3` next to `kimi-code/k3` — so each model is its
/// own group. The label is also searchable, which is how "k3" finds it.
pub(super) fn groups_in(config: &str) -> Vec<ModelGroup> {
    let mut groups: Vec<ModelGroup> = Vec::new();
    let mut alias: Option<String> = None;
    let mut display: Option<String> = None;

    let mut flush = |alias: &mut Option<String>, display: &mut Option<String>| {
        if let Some(id) = alias.take() {
            let label = display.take().unwrap_or_else(|| id.clone());
            groups.push(ModelGroup {
                label,
                models: vec![id],
            });
        }
        *display = None;
    };

    for line in config.lines() {
        let line = line.trim();
        // A new section ends whichever model section was open, including
        // `[thinking]` and the provider blocks — otherwise a `display_name`
        // from somewhere else could attach to the previous model.
        if line.starts_with('[') {
            flush(&mut alias, &mut display);
            alias = model_alias(line);
            continue;
        }
        if alias.is_some()
            && let Some(name) = quoted_value(line, "display_name")
        {
            display = Some(name);
        }
    }
    flush(&mut alias, &mut display);
    groups
}

/// The alias in a `[models."vendor/model"]` header, if that is what this is.
fn model_alias(line: &str) -> Option<String> {
    let inner = line.strip_prefix("[models.")?.strip_suffix(']')?;
    let alias = inner.strip_prefix('"')?.strip_suffix('"')?;
    // A quote inside the alias means this is not the simple form this scanner
    // understands. Skipping it loses one menu entry; guessing loses trust in
    // every other one.
    (!alias.is_empty() && !alias.contains('"')).then(|| alias.to_string())
}

/// The value of `key = "..."`, when the line is exactly that.
fn quoted_value(line: &str, key: &str) -> Option<String> {
    let rest = line
        .strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim();
    let value = rest.strip_prefix('"')?.strip_suffix('"')?;
    (!value.contains('"')).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real shape of a Kimi config, including the sections that must not
    /// contribute models.
    const REAL: &str = r#"
default_model = "kimi-code/k3"

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
display_name = "K2.7 Coding"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
display_name = "K3"
support_efforts = [ "low", "high", "max" ]

[models."kimi-code/k3-256k"]
provider = "managed:kimi-code"
display_name = "K3-256k"

[thinking]
enabled = true
"#;

    #[test]
    fn every_configured_model_is_offered_with_its_display_name() {
        let groups = groups_in(REAL);
        let offered: Vec<(&str, &str)> = groups
            .iter()
            .map(|group| (group.models[0].as_str(), group.label.as_str()))
            .collect();
        assert_eq!(
            offered,
            [
                ("kimi-code/kimi-for-coding", "K2.7 Coding"),
                ("kimi-code/k3", "K3"),
                ("kimi-code/k3-256k", "K3-256k"),
            ],
            "the menu does not match the models this config defines"
        );
    }

    /// Only `[models.*]` sections contribute, and a display name cannot leak
    /// across a section boundary.
    #[test]
    fn other_sections_contribute_nothing() {
        let groups = groups_in(REAL);
        assert!(
            groups.iter().all(|group| group.models[0].contains('/')),
            "a provider or thinking section was offered as a model: {groups:?}"
        );
        assert_eq!(groups.len(), 3, "{groups:?}");
    }

    /// No config is an empty menu, not a panic and not an invented list.
    #[test]
    fn an_unreadable_config_offers_nothing_rather_than_guessing() {
        assert!(groups_in("").is_empty());
        assert!(groups_in("garbage without any sections").is_empty());
        assert!(groups_in("[models.\"\"]\ndisplay_name = \"empty\"").is_empty());
    }

    /// A model with no display name still appears, under its own id.
    #[test]
    fn a_model_without_a_display_name_is_still_offered() {
        let groups = groups_in("[models.\"kimi-code/plain\"]\nprovider = \"x\"");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].models[0], "kimi-code/plain");
        assert_eq!(groups[0].label, "kimi-code/plain");
    }

    /// The scan finds and parses a Kimi config under a fixture home.
    ///
    /// A host-dependent skip left CI green while `config_under` / `groups_in`
    /// could be broken. Env mutation is forbidden by the workspace `unsafe_code`
    /// lint, so the fixture home is passed to `config_under` directly — the
    /// same join `config_path` uses after reading HOME/USERPROFILE.
    #[test]
    fn the_real_local_config_parses_if_present() {
        let home = std::env::temp_dir().join(format!(
            "bugsleuth-kimi-config-{}",
            std::process::id()
        ));
        let config_dir = home.join(".kimi-code");
        std::fs::create_dir_all(&config_dir).expect("fixture config dir");
        let config = config_dir.join("config.toml");
        std::fs::write(
            &config,
            r#"default_model = "kimi-code/k3"
[models."kimi-code/k3"]
provider = "managed:kimi-code"
"#,
        )
        .expect("write fixture config");
        let path = config_under(home.clone());
        assert_eq!(path, config, "config_under must resolve the installer layout");
        assert!(path.is_file(), "fixture config was not written at {path:?}");
        let text = std::fs::read_to_string(&path).expect("read config");
        let groups = groups_in(&text);
        assert!(!groups.is_empty(), "fixture config produced no groups");
        let _ = std::fs::remove_dir_all(&home);
    }
}
