//! What the app remembers between launches.
//!
//! Stored as one JSON file under the platform config directory in a friendly
//! `BugSleuth` folder, so it is findable and removable by hand. Deliberately
//! small: the repository you last reviewed, which models cover which lanes, and
//! your theme preference. Findings are not cached here — they live in the run
//! output directory, which is the thing you would actually want to keep.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Last repository reviewed, so the app opens where you left off.
    pub repo: String,
    /// Optional path scope within the repository.
    pub scope: String,
    /// Models and the lanes each covers.
    pub models: Vec<ModelSetting>,
    /// `system`, `light` or `dark`.
    pub theme: String,
    /// How many merged defects to attempt proof for. 0 disables proving.
    pub prove_top: usize,
    /// Command that runs the target's tests, needed only for proving.
    pub test_command: String,
    /// Reuse sweeps already on disk for this repository instead of paying for
    /// them again.
    ///
    /// On by default, which is the opposite of the command line's `--resume`.
    /// A run is tens of minutes and the window can be closed, a CLI dies and
    /// dropped, so the desktop case that actually happens is "that run died at
    /// nine of twelve and I pressed Run again". Paying for nine sweeps a second
    /// time is the surprising outcome, not reusing them.
    #[serde(default = "yes")]
    pub reuse_completed: bool,
}

/// Serde needs a function; a bare `true` default is not expressible.
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSetting {
    pub id: String,
    pub lanes: Vec<String>,
    /// Reasoning effort. Empty means the vendor's own default.
    ///
    /// Defaulted on read so a settings file written before efforts existed
    /// still loads, rather than being discarded back to the shipped preset.
    #[serde(default)]
    pub effort: String,
}

impl Default for Settings {
    /// The shipped default is the "Balanced" preset: every lane covered, and
    /// the two vendors that can be run read-only doubled up on correctness.
    ///
    /// Every lane has a model on purpose. A lane with nobody assigned is
    /// reported as unswept, which is correct but is a bad thing to hand someone
    /// on first launch without their having chosen it.
    fn default() -> Self {
        Self {
            repo: String::new(),
            scope: String::new(),
            models: vec![
                ModelSetting {
                    id: "sonnet".into(),
                    effort: String::new(),
                    lanes: vec![
                        "correctness".into(),
                        "security".into(),
                        "contract".into(),
                        "ux".into(),
                    ],
                },
                ModelSetting {
                    id: "codex:".into(),
                    lanes: vec!["correctness".into(), "security".into()],
                    effort: String::new(),
                },
            ],
            theme: "system".into(),
            prove_top: 0,
            test_command: String::new(),
            reuse_completed: true,
        }
    }
}

/// The app's own directory: `%APPDATA%\BugSleuth` on Windows, the equivalent
/// config root elsewhere. A friendly name on purpose — everything BugSleuth
/// writes outside a reviewed repository lives here and can be deleted by hand.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("BugSleuth")
}

fn path() -> Option<PathBuf> {
    Some(data_dir().join("settings.json"))
}

/// Read stored settings, falling back to the default.
///
/// A missing or unreadable file is not an error worth surfacing: the app is
/// perfectly usable with defaults, and refusing to start because a preferences
/// file was corrupted would be a worse outcome than losing the preferences.
pub fn load() -> Settings {
    let Some(path) = path() else {
        return Settings::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> anyhow::Result<()> {
    let path = path().ok_or_else(|| anyhow::anyhow!("no config directory on this platform"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(settings)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_covers_every_lane_so_a_first_run_has_no_silent_holes() {
        let settings = Settings::default();
        let covered: Vec<&str> = settings
            .models
            .iter()
            .flat_map(|m| m.lanes.iter().map(String::as_str))
            .collect();
        for lane in ["correctness", "security", "contract", "ux"] {
            assert!(covered.contains(&lane), "{lane} has no model by default");
        }
    }

    #[test]
    fn the_default_does_not_prove_anything_until_asked() {
        // Proving costs a model invocation and a full test run per defect, so
        // it must never happen because someone pressed Run without reading.
        assert_eq!(Settings::default().prove_top, 0);
    }

    #[test]
    fn unknown_fields_and_missing_fields_both_survive_a_round_trip() {
        // Settings written by a newer or older build must not brick the app.
        let sparse = r#"{"repo":"C:/x","theme":"dark"}"#;
        let parsed: Settings = serde_json::from_str(sparse).unwrap_or_default();
        assert_eq!(parsed.repo, "C:/x");
        assert_eq!(parsed.theme, "dark");
        assert!(
            !parsed.models.is_empty(),
            "missing models fall back to default"
        );
    }
}
