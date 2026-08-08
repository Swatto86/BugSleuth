//! Turning a finished run into what the window is handed.
//!
//! The gap lines and the fix prompt: everything that happens *after* the sweeps.
//! Kept apart from the commands that start them because it is a different job.

use bugsleuth_engine::orchestrate;

/// Every lane nobody reviewed, as one line each.
pub(crate) fn gap_lines(report: &orchestrate::RunReport) -> Vec<String> {
    report
        .gaps
        .iter()
        .map(|gap| {
            format!(
                "{} lane, by {} — {}",
                gap.lane,
                gap.model.as_deref().unwrap_or("nobody"),
                gap.reason
            )
        })
        .collect()
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
