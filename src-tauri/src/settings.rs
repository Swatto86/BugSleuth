//! What the app remembers between launches.
//!
//! Stored as one JSON file under the platform config directory in a friendly
//! `BugSleuth` folder, so it is findable and removable by hand. Deliberately
//! small: the repository you last reviewed, which models cover which lanes, and
//! your theme preference. Findings are not cached here — they live in the run
//! output directory, which is the thing you would actually want to keep.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
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
    /// Model that re-grades every severity once the sweeps are merged, with the
    /// whole list in view.
    ///
    /// On by default with the cheapest model, because severity is the only
    /// thing that orders the report and each sweep grades its own findings in
    /// isolation — measured wrong 6 times in 14. Empty turns the pass off and
    /// keeps whatever each model called its own finding.
    #[serde(default = "cheapest")]
    pub triage_model: String,
    /// The model that applies the fixes when asked, as a `vendor:model` spec.
    ///
    /// Separate from the sweep matrix on purpose: finding a defect and fixing it
    /// are different jobs, and the one you would spend a cheap model on to read
    /// every lane is not necessarily the one you want editing your code. Empty
    /// until chosen, and the button refuses rather than guessing.
    #[serde(default)]
    pub apply_model: String,
    /// Reasoning effort for that model. Empty means the vendor's own default.
    ///
    /// Its own field rather than part of the spec, because effort is not part of
    /// a model id: the same id takes different levels on different vendors, and
    /// packing it into the string would send `opus:high` to a CLI as a model name.
    #[serde(default)]
    pub apply_effort: String,
    /// Push what an apply committed to the branch's existing upstream.
    ///
    /// Off by default, and deliberately not remembered as a convenience: every
    /// other thing an apply does is undone with `git reset`, and this is the
    /// one that cannot be. It only ever pushes the current branch where it
    /// already goes — never a force, never a guessed remote — and refuses
    /// outright if any commit still credits a tool for the work.
    #[serde(default)]
    pub push_after_apply: bool,
    /// After a successful push, tag the published commits so the repository's
    /// own CI cuts a release from them.
    ///
    /// Off by default, and only ever acted on when the push succeeded: a tag is
    /// the trigger for a release pipeline, so it is the one step here with
    /// consequences beyond the repository — a build, artifacts, and a published
    /// release someone else may download. The version is read from the branch's
    /// own most recent `v*` tag and the patch bumped; a repository with no such
    /// tag, or one that names releases some other way, is left alone rather than
    /// given a scheme it never chose.
    #[serde(default)]
    pub tag_release_after_push: bool,
}

/// Serde needs a function; a bare string default is not expressible.
fn cheapest() -> String {
    "haiku".to_string()
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
    /// How many times to sweep each lane with this model. One by default;
    /// more is deliberate repetition, which measurably finds more.
    #[serde(default = "one_pass")]
    pub passes: usize,
}

fn one_pass() -> usize {
    1
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
                    passes: 1,
                    lanes: vec![
                        "correctness".into(),
                        "security".into(),
                        "contract".into(),
                        "ux".into(),
                        "gate".into(),
                    ],
                },
                ModelSetting {
                    id: "codex:".into(),
                    lanes: vec!["correctness".into(), "security".into()],
                    effort: String::new(),
                    passes: 1,
                },
            ],
            theme: "system".into(),
            prove_top: 0,
            test_command: String::new(),
            reuse_completed: true,
            triage_model: cheapest(),
            // Nothing by default: applying fixes writes to the user's own
            // checkout, and a model nobody chose is not something to default to.
            apply_model: String::new(),
            apply_effort: String::new(),
            push_after_apply: false,
            tag_release_after_push: false,
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

/// Read stored settings.
///
/// Only a *missing* file is first-launch and yields the defaults. A file that
/// cannot be read or does not parse is an error, propagated so the app can tell
/// the user rather than silently rendering defaults — which the frontend then
/// persists, overwriting the recoverable file and losing the configuration for
/// good.
fn load_from(path: &Path) -> anyhow::Result<Settings> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot read saved settings from {}", path.display()));
        }
    };

    serde_json::from_str(&text)
        .with_context(|| format!("saved settings at {} are not valid JSON", path.display()))
}

pub fn load() -> anyhow::Result<Settings> {
    let path = path().ok_or_else(|| anyhow::anyhow!("no config directory on this platform"))?;
    load_from(&path)
}

pub fn save(settings: &Settings) -> anyhow::Result<()> {
    let path = path().ok_or_else(|| anyhow::anyhow!("no config directory on this platform"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // `fs::write` truncates first, so a failure partway through — a full disk, a
    // process killed — left an empty or half-written settings file where a good
    // one had been, and every configuration in it was gone on restart. Losing
    // settings silently is the exact incident this module's own error reporting
    // was added for; it should not have been possible to lose them this way at
    // the same time.
    bugsleuth_engine::atomic::write(&path, serde_json::to_string_pretty(settings)?)?;
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
        for lane in ["correctness", "security", "contract", "ux", "gate"] {
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
}
