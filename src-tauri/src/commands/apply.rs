//! Handing a finished review to a model that will act on it.
//!
//! The prompt is read from disk rather than taken from the window. The window
//! holds a copy — it is what the Copy button gives you — but it arrives here as
//! a string a webview chose, and this is the one command whose argument becomes
//! instructions to an agent with write access. The file the run wrote is the
//! only source that cannot have been substituted on the way.

use std::time::Duration;

use tauri::{Emitter, Manager};

use super::RunControl;
use super::run::{checked_repo, run_output_dir};
use crate::settings::Settings;

/// How long one apply may take, and how many turns it gets.
///
/// Far more generous than a sweep, because the work is: reading each defect,
/// changing the code, writing a test and running it, for every defect in the
/// report. Being cut short is not a disaster — everything done so far is in git
/// and is reported — but it is a waste, so the ceiling is high enough that
/// hitting it means something is wrong rather than that the list was long.
const APPLY_TIMEOUT: Duration = Duration::from_secs(7200);
const APPLY_MAX_TURNS: u32 = 300;

/// Apply the last run's fix prompt with the chosen model.
///
/// Returns immediately; the result arrives as an `apply-finished` event, exactly
/// as a run does, because this takes minutes to hours.
#[tauri::command]
pub async fn apply_fixes(
    app: tauri::AppHandle,
    control: tauri::State<'_, RunControl>,
    settings: Settings,
) -> Result<(), String> {
    if control.running() {
        return Err(
            "a review is running — applying fixes now would edit the code it is reading"
                .to_string(),
        );
    }
    if control.applying() {
        return Err("fixes are already being applied".to_string());
    }

    let repo = checked_repo(&settings.repo)?;
    let model = settings.apply_model.trim().to_string();
    if model.is_empty() {
        return Err("choose a provider and model to apply the fixes with".to_string());
    }

    let prompt_path = run_output_dir(&repo).join("fix-prompt.md");
    let prompt = std::fs::read_to_string(&prompt_path).map_err(|e| {
        format!(
            "no fix prompt for this repository at {}: {e}. Run a review first.",
            prompt_path.display()
        )
    })?;

    control.set_applying(true);
    tauri::async_runtime::spawn(async move {
        let report = bugsleuth_engine::apply::apply(bugsleuth_engine::apply::ApplyRequest {
            repo: &repo,
            model: &model,
            effort: "",
            prompt: &prompt,
            timeout: APPLY_TIMEOUT,
            max_turns: APPLY_MAX_TURNS,
        })
        .await;

        let payload = match report {
            Ok(report) => serde_json::json!({
                "ok": true,
                "text": describe(&report),
                "changed": report.changed_files,
            }),
            Err(error) => serde_json::json!({
                "ok": false,
                "text": error.to_string(),
                "changed": Vec::<String>::new(),
            }),
        };
        let _ = app.emit("apply-finished", payload);
        if let Some(control) = app.try_state::<RunControl>() {
            control.set_applying(false);
        }
    });

    Ok(())
}

/// The model's account, under what git says actually happened.
///
/// git first and deliberately: the account is a claim, and a reader who has
/// already seen the file list reads the claim against it rather than instead
/// of it. A model that reports three fixes and touched nothing is the case this
/// ordering exists for.
fn describe(report: &bugsleuth_engine::apply::ApplyReport) -> String {
    let mut out = String::new();
    if report.changed_files.is_empty() {
        out.push_str(
            "git reports no files changed. Whatever the model says below, nothing in \
                      the repository is different.\n\n",
        );
    } else {
        out.push_str(&format!(
            "{} file{} changed, in {} commit{}:\n",
            report.changed_files.len(),
            if report.changed_files.len() == 1 {
                ""
            } else {
                "s"
            },
            report.commits,
            if report.commits == 1 { "" } else { "s" },
        ));
        for file in &report.changed_files {
            out.push_str(&format!("  {file}\n"));
        }
        out.push_str(
            "\nNothing here has been verified. Read the diff before you keep it: `git diff` for \
             what is uncommitted, `git log -p` for what was committed.\n\n",
        );
    }
    out.push_str("The model's own account:\n\n");
    out.push_str(report.text.trim());
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_engine::apply::ApplyReport;

    #[test]
    fn a_model_that_changed_nothing_cannot_report_success_unchallenged() {
        // The dangerous outcome: a confident "fixed all seven" over a tree git
        // says is untouched. The window shows this text, so the contradiction
        // has to be in it.
        let text = describe(&ApplyReport {
            text: "I fixed all seven defects.".into(),
            changed_files: vec![],
            commits: 0,
        });
        assert!(text.contains("no files changed"));
        assert!(text.contains("fixed all seven"));
    }

    #[test]
    fn what_changed_is_listed_before_what_was_claimed() {
        let text = describe(&ApplyReport {
            text: "done".into(),
            changed_files: vec!["src/a.rs".into(), "src/b.rs".into()],
            commits: 2,
        });
        assert!(text.contains("2 files changed, in 2 commits"));
        assert!(text.contains("src/b.rs"));
        assert!(
            text.find("src/a.rs") < text.find("The model's own account"),
            "the claim is shown before the evidence"
        );
        // Nobody has checked these edits, and the text must not imply otherwise.
        assert!(text.contains("git diff"));
    }
}
