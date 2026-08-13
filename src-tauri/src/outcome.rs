//! Turning a finished run into what the window is handed.
//!
//! The gap lines and the fix prompt: everything that happens *after* the sweeps.
//! Kept apart from the commands that start them because it is a different job.

use bugsleuth_engine::orchestrate;

/// Every lane or repository path nobody reviewed, as one line each.
pub(crate) fn gap_lines(report: &orchestrate::RunReport) -> Vec<String> {
    report.not_reviewed()
}

/// The defects as a prompt for a coding agent.
pub(crate) fn fix_prompt(repo: &std::path::Path, report: &orchestrate::RunReport) -> String {
    bugsleuth_engine::handoff::prompt(
        &repo.display().to_string(),
        &report.ranked,
        &gap_lines(report),
        report.swept.len(),
    )
}

/// Everything the window is told about a finished run.
///
/// Extracted from `start_run` at the function-length limit, along the seam the
/// module note already draws: that command owns the run's lifetime, and this is
/// what to say once it is over.
///
/// `cancelled` is carried in rather than derived from `report`. Stopping
/// mid-run yields an `Ok(RunReport)` with cancellation gaps while stopping
/// during pre-check yields an `Err`, so the same Stop was reported as
/// "Finished" or "Run failed" purely by timing. `ok` keeps its original meaning
/// — a report is available — because a stopped review's partial report is still
/// worth copying.
pub(crate) fn run_payload(
    report: anyhow::Result<orchestrate::RunReport>,
    cancelled: bool,
    repo: &std::path::Path,
    out_dir: &std::path::Path,
) -> serde_json::Value {
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            return serde_json::json!({
                "ok": false,
                "complete": false,
                "cancelled": cancelled,
                "text": error.to_string(),
            });
        }
    };
    let complete = report.coverage_complete();
    let mut text = report.to_text();

    // The prompt is the thing that gets used, so it is written to disk as well
    // as handed to the window. A run is tens of minutes and the window can be
    // closed; losing the output to a stray click would be the worst ending.
    let prompt = fix_prompt(repo, &report);
    // A save that half-failed used to be swallowed by `.ok()`, so the window
    // said Finished over a handoff that was missing files. Both a hard failure
    // and a partial one are surfaced now.
    let (prompt_path, save_error) = match bugsleuth_engine::handoff::write_all(
        out_dir,
        &repo.display().to_string(),
        &report.ranked,
        &gap_lines(&report),
        report.swept.len(),
    ) {
        Ok(written) => {
            let warning = (!written.warnings.is_empty()).then(|| {
                format!(
                    "The review finished, but some fix prompts were not saved: {}",
                    written.warnings.join("; ")
                )
            });
            (Some(written.bundle.display().to_string()), warning)
        }
        Err(error) => (
            None,
            Some(format!(
                "The review finished, but its fix prompt could not be saved to {}: {error}",
                out_dir.display()
            )),
        ),
    };
    if let Some(warning) = &save_error {
        text.push_str("\n\n");
        text.push_str(warning);
    }
    serde_json::json!({
        "ok": true,
        "complete": complete,
        "cancelled": cancelled,
        "text": text,
        "prompt": prompt,
        "promptPath": prompt_path,
        "saveError": save_error,
        "findings": crate::payload::findings(&repo.display().to_string(), &report),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stop must be reported as Stop on both paths it can take.
    ///
    /// A mid-run Stop returns `Ok(RunReport)` with cancellation gaps and one
    /// during pre-check returns `Err`, so the window saw only `ok` and called
    /// the same action "Finished" or "Run failed" by timing alone.
    #[test]
    fn a_stopped_run_says_so_whether_or_not_it_produced_a_report() {
        let dir = std::env::temp_dir()
            .join("bugsleuth-payload")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");

        let partial = run_payload(
            Ok(orchestrate::RunReport {
                ranked: vec![],
                triage: Default::default(),
                swept: vec![],
                gaps: vec![],
                cancelled: true,
            }),
            true,
            &dir,
            &dir,
        );
        assert_eq!(partial["cancelled"], serde_json::json!(true));
        // Still `ok`: a stopped review's partial report is worth copying.
        assert_eq!(partial["ok"], serde_json::json!(true));
        assert_eq!(partial["complete"], serde_json::json!(true));

        let precheck = run_payload(
            Err(anyhow::anyhow!("Provider pre-check stopped")),
            true,
            &dir,
            &dir,
        );
        assert_eq!(precheck["cancelled"], serde_json::json!(true));
        assert_eq!(precheck["ok"], serde_json::json!(false));
        assert_eq!(precheck["complete"], serde_json::json!(false));

        // And a run that really finished is not marked stopped.
        let finished = run_payload(
            Ok(orchestrate::RunReport {
                ranked: vec![],
                triage: Default::default(),
                swept: vec![],
                gaps: vec![],
                cancelled: false,
            }),
            false,
            &dir,
            &dir,
        );
        assert_eq!(finished["cancelled"], serde_json::json!(false));
        assert_eq!(finished["complete"], serde_json::json!(true));

        let incomplete = run_payload(
            Ok(orchestrate::RunReport {
                ranked: vec![],
                triage: Default::default(),
                swept: vec![],
                gaps: vec![orchestrate::Gap {
                    lane: bugsleuth_domain::Lane::Contract,
                    model: Some("test-model".to_string()),
                    reason: "rate limited".to_string(),
                }],
                cancelled: false,
            }),
            false,
            &dir,
            &dir,
        );
        assert_eq!(incomplete["ok"], serde_json::json!(true));
        assert_eq!(incomplete["complete"], serde_json::json!(false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
