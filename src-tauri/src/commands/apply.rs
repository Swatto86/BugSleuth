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
    let repo = checked_repo(&settings.repo)?;
    let model = settings.apply_model.trim().to_string();
    if model.is_empty() {
        return Err("choose a provider and model to apply the fixes with".to_string());
    }

    let effort = settings.apply_effort.trim().to_string();
    let prompt_path = run_output_dir(&repo).join("fix-prompt.md");
    let prompt = std::fs::read_to_string(&prompt_path).map_err(|e| {
        format!(
            "no fix prompt for this repository at {}: {e}. Run a review first.",
            prompt_path.display()
        )
    })?;

    // Reserve the applying state atomically, after the fallible setup above and
    // immediately before spawning. A review reads the tree while this rewrites
    // it, so the two must never overlap — and a single check-then-set could let
    // a run start in the gap. Every early return before here has reserved
    // nothing, so none leaks the state.
    control.try_start_apply()?;
    tauri::async_runtime::spawn(async move {
        let report = bugsleuth_engine::apply::apply(bugsleuth_engine::apply::ApplyRequest {
            repo: &repo,
            model: &model,
            effort: &effort,
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
            control.finish_apply();
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
        let files = report.changed_files.len();
        out.push_str(&format!(
            "{files} file{} changed, {}:\n",
            if files == 1 { "" } else { "s" },
            match report.commits {
                0 => "none of it committed".to_string(),
                1 => "in 1 commit".to_string(),
                n => format!("in {n} commits"),
            },
        ));
        for file in &report.changed_files {
            out.push_str(&format!("  {file}\n"));
        }
        out.push_str(
            "\nNothing here has been verified. Read the diff before you keep it: `git diff` for \
             what is uncommitted, `git log -p` for what was committed.\n\n",
        );
    }
    // Said plainly, because rewriting someone's commits without telling them is
    // worse than leaving the trailer on.
    if !report.stripped.is_empty() {
        out.push_str(&format!(
            "{}, so this tool does not sign your history:
",
            match report.stripped.len() {
                1 => "Removed a tool authorship trailer from 1 commit".to_string(),
                n => format!("Removed tool authorship trailers from {n} commits"),
            }
        ));
        for subject in &report.stripped {
            out.push_str(&format!(
                "  {subject}
"
            ));
        }
        out.push_str(
            "
The changes themselves are untouched — only the trailer lines are gone.

",
        );
    }

    // What could not be rewritten, because it was already published or HEAD was
    // detached. Reported rather than forced: forking history to tidy a trailer
    // is a worse outcome than the trailer.
    if !report.attributed.is_empty() {
        out.push_str(&format!(
            "{}:\n",
            match report.attributed.len() {
                1 => "1 commit credits a tool for the work".to_string(),
                n => format!("{n} commits credit a tool for the work"),
            }
        ));
        for subject in &report.attributed {
            out.push_str(&format!("  {subject}\n"));
        }
        out.push_str(
            "\nStrip the trailer before pushing if your project does not want it — once it is \
             published it is in the history for good.\n\n",
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
            stripped: vec![],
            attributed: vec![],
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
            stripped: vec![],
            attributed: vec![],
        });
        assert!(text.contains("2 files changed, in 2 commits"), "{text}");
        assert!(text.contains("src/b.rs"));
        assert!(
            text.find("src/a.rs") < text.find("The model's own account"),
            "the claim is shown before the evidence"
        );
        // Uncommitted work is the common case — the prompt asks for a commit per
        // defect, but a model that stops early has changed files and committed
        // none of them, and "in 0 commits" read as a count nobody had noticed
        // was zero.
        let uncommitted = describe(&ApplyReport {
            text: "done".into(),
            changed_files: vec!["src/a.rs".into()],
            commits: 0,
            stripped: vec![],
            attributed: vec![],
        });
        assert!(
            uncommitted.contains("1 file changed, none of it committed"),
            "{uncommitted}"
        );

        // Nobody has checked these edits, and the text must not imply otherwise.
        assert!(text.contains("git diff"));
    }

    #[test]
    fn a_commit_that_credits_the_tool_is_named_before_it_is_pushed() {
        // The prompt tells the model not to add an authorship trailer and every
        // CLI adds one by default, so this is reported from the commits rather
        // than taken on trust. Seven of them reached a real repository whose
        // published history had none, and were caught by luck.
        let text = describe(&ApplyReport {
            text: "done".into(),
            changed_files: vec!["src/a.rs".into()],
            commits: 1,
            stripped: vec![],
            attributed: vec!["fix(auth): stop demanding re-sign-in".into()],
        });
        assert!(text.contains("1 commit credits a tool"), "{text}");
        assert!(text.contains("fix(auth): stop demanding re-sign-in"));
        assert!(text.contains("published it is in the history for good"));
        // Said before the model's account, which is the part someone skims.
        assert!(text.find("credits a tool") < text.find("The model's own account"));

        // Plural, and only when there is something to say.
        let two = describe(&ApplyReport {
            text: "done".into(),
            changed_files: vec!["a".into()],
            commits: 2,
            stripped: vec![],
            attributed: vec!["one".into(), "two".into()],
        });
        assert!(two.contains("2 commits credit a tool"), "{two}");
        let clean = describe(&ApplyReport {
            text: "done".into(),
            changed_files: vec!["a".into()],
            commits: 1,
            stripped: vec![],
            attributed: vec![],
        });
        assert!(!clean.contains("credit"), "{clean}");
    }
}
