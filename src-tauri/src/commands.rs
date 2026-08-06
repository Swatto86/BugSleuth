//! The frontend's entire surface.
//!
//! Every command is a deserialize, a call into `bugsleuth-engine`, and a
//! serialize. Nothing here decides anything; if a body starts growing a
//! judgement, that judgement belongs in the engine where it can be tested
//! without a window.
//!
//! Arguments arrive from a webview and are untrusted. In particular a
//! repository path is a string the frontend chose, so it is canonicalized and
//! checked to be a real directory before anything acts on it — the frontend has
//! no filesystem permission of its own precisely so that this is the only door.

use serde::Serialize;
use tauri::Manager;

use crate::settings::{self, Settings};

pub mod apply;
pub mod run;
pub mod saved;
pub use run::RunControl;

/// Errors cross to the frontend as plain strings. The webview cannot act on a
/// typed error, and a message a person can read is worth more than a variant.
type CommandResult<T> = Result<T, String>;

#[derive(Serialize)]
pub struct VendorStatus {
    pub name: String,
    pub available: bool,
    pub detail: String,
}

/// Which provider CLIs can be started. Free — starts no model.
#[tauri::command]
pub async fn preflight() -> Vec<VendorStatus> {
    bugsleuth_engine::sweep::probe_all()
        .await
        .into_iter()
        .map(|(name, result)| match result {
            Ok(version) => VendorStatus {
                name: name.to_string(),
                available: true,
                detail: version,
            },
            Err(error) => VendorStatus {
                name: name.to_string(),
                available: false,
                detail: error,
            },
        })
        .collect()
}

/// Prove each vendor is actually signed in, by asking it something.
///
/// Separate from `preflight`, and deliberately not run at startup. Preflight is
/// free and answers "can this CLI start"; this costs one trivial model call per
/// vendor and answers the question that actually decides whether a run will
/// work. A green preflight beside a signed-out CLI is how someone commits forty
/// minutes and real quota to a run that could never have finished.
#[tauri::command]
pub async fn check_signin() -> Vec<VendorStatus> {
    bugsleuth_engine::sweep::check_signin()
        .await
        .into_iter()
        .map(|(name, state)| VendorStatus {
            available: state.usable(),
            detail: state.describe(name),
            name: name.to_string(),
        })
        .collect()
}

#[tauri::command]
pub fn load_settings() -> Settings {
    settings::load()
}

#[tauri::command]
pub fn save_settings(settings: Settings) -> CommandResult<()> {
    settings::save(&settings).map_err(|e| e.to_string())
}

/// Native folder picker. The frontend has no filesystem permission, so this is
/// how a repository gets chosen.
#[tauri::command]
pub async fn pick_directory(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    rx.recv().ok().flatten().map(|folder| folder.to_string())
}

/// Exit the application for real.
///
/// The window's close button hides to the tray, so without this the only way
/// out is the tray menu — and if the tray ever fails to appear, there would be
/// no way out at all short of the task manager. A second, keyboard-reachable
/// exit is cheap insurance against an unrecoverable state.
#[tauri::command]
pub fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

/// Reveal the window once the frontend has mounted and themed itself.
#[tauri::command]
pub fn frontend_ready(app: tauri::AppHandle) {
    crate::reveal(&app);
    let _ = app.get_webview_window("main");
}

#[cfg(test)]
mod tests {
    use super::run::{checked_repo, run_output_dir, to_config};
    use super::*;
    use crate::outcome::fix_prompt;
    use bugsleuth_engine::{orchestrate, plan};

    #[test]
    fn an_empty_repository_path_is_refused_with_something_actionable() {
        let error = checked_repo("   ").unwrap_err();
        assert!(error.contains("choose a repository"));
    }

    #[test]
    fn a_path_that_does_not_exist_is_refused_rather_than_used() {
        assert!(checked_repo("Z:/definitely/not/here").is_err());
    }

    #[test]
    fn a_real_directory_resolves_without_the_extended_length_prefix() {
        // git rejects the \\?\ form that canonicalize produces on Windows, and
        // every worktree operation would fail on it.
        let resolved =
            checked_repo(".").expect("checked_repo should resolve the current directory");
        assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn settings_map_onto_the_engine_config_without_losing_lanes() {
        let settings = Settings::default();
        let config = to_config(&settings);
        assert_eq!(config.models.len(), settings.models.len());
        let planned = plan::plan(&config).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            planned.uncovered.is_empty(),
            "the default settings should leave no lane unswept"
        );
    }

    #[test]
    fn the_fix_prompt_carries_the_repo_and_every_unreviewed_lane() {
        // The app-side half of the deliverable. It cannot be observed without a
        // window, so it is checked here instead: an agent handed this must learn
        // which repository it is working on and which lanes nobody looked at.
        // Omitting either turns an incomplete review into an apparently
        // complete one.
        let report = orchestrate::RunReport {
            ranked: vec![],
            triage: Default::default(),
            swept: vec![],
            gaps: vec![
                orchestrate::Gap {
                    lane: bugsleuth_domain::Lane::Security,
                    model: None,
                    reason: "no model is assigned to this lane".to_string(),
                },
                orchestrate::Gap {
                    lane: bugsleuth_domain::Lane::Ux,
                    model: Some("kilo:".to_string()),
                    reason: "the kilo CLI exited with code 1".to_string(),
                },
            ],
        };
        let prompt = fix_prompt(std::path::Path::new("C:/x/my-repo"), &report);

        assert!(prompt.contains("my-repo"), "the repository is not named");
        assert!(
            prompt.contains("Security"),
            "an unassigned lane is not listed"
        );
        assert!(
            prompt.contains("nobody"),
            "an unassigned lane has no owner shown"
        );
        assert!(prompt.contains("Ux") || prompt.contains("UX"));
        assert!(
            prompt.contains("kilo:"),
            "a failed sweep does not name its model"
        );
        assert!(
            prompt.contains("not evidence that they are clean"),
            "nothing warns that the unreviewed lanes are not known-good"
        );
    }

    #[test]
    fn two_checkouts_with_the_same_folder_name_do_not_share_a_run_directory() {
        // A worktree beside the original, or a clone under a different parent,
        // has the same leaf name. Sharing a run directory means resume hands
        // one of them the other's sweeps and the report states the wrong
        // provenance - the same lossy-key defect as the report filenames.
        let a = run_output_dir(std::path::Path::new("C:/work/bugsleuth"));
        let b = run_output_dir(std::path::Path::new("C:/scratch/bugsleuth"));
        assert_ne!(a, b);
        // The leaf stays visible so a person can still tell which is which.
        assert!(a.to_string_lossy().contains("bugsleuth"));
        // And the same path always resolves to the same directory, or resume
        // would never find anything.
        assert_eq!(a, run_output_dir(std::path::Path::new("C:/work/bugsleuth")));
    }
}
