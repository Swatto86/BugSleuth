//! Keeping each sweep on disk, and picking one back up.
//!
//! A sweep costs real subscription quota and can take tens of minutes. Writing
//! each one out as it lands is what makes a run that dies at unit nine of twelve
//! recoverable instead of a total loss.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::RunOptions;
use crate::plan::Unit;
use crate::report::{LaneReport, Status};

/// A previous successful sweep for this unit, if resuming and one exists.
///
/// A file that cannot be read or parsed is treated as absent rather than as an
/// error: the likeliest cause is a run killed mid-write, and the right response
/// to a truncated report is to sweep again, not to refuse to start.
pub(super) fn reusable(unit: &Unit, options: &RunOptions<'_>) -> Option<LaneReport> {
    if !options.resume {
        return None;
    }
    let dir = options.out_dir?;
    read_swept(&dir.join(file_name_for(unit)))
        .filter(|report| same_scope(report, options.scope))
        // Reports written before the encoding changed are still worth tens of
        // minutes each, so the old name is tried too — but only after the
        // current one, and only if the report says it is the right sweep. The
        // old encoding was lossy, which is exactly why it is not trusted to
        // identify anything on its own.
        .or_else(|| {
            let legacy = read_swept(&dir.join(legacy_file_name_for(unit)))?;
            // The scope check belongs on this path too. Adding it only to the
            // branch above would leave the whole defect reachable through the
            // legacy name, which is exactly the "fixed the first sink, missed
            // the second" shape the correctness mandate hunts for.
            let same_sweep = legacy.lane == unit.lane.title()
                && legacy.model.ends_with(model_of(&unit.model))
                && same_scope(&legacy, options.scope);
            same_sweep.then_some(legacy)
        })
}

/// A report at `path`, if it parses and records a sweep that actually ran.
///
/// A file that cannot be read or parsed is treated as absent rather than as an
/// error: the likeliest cause is a run killed mid-write, and the right response
/// to a truncated report is to sweep again, not to refuse to start.
fn read_swept(path: &Path) -> Option<LaneReport> {
    let text = std::fs::read_to_string(path).ok()?;
    let report: LaneReport = serde_json::from_str(&text).ok()?;
    // A failed sweep is retried. The usual reason a run died is a rate limit,
    // which is exactly the case worth attempting again.
    matches!(report.status, Status::Swept { .. }).then_some(report)
}

/// Whether a stored report reviewed the same scope this run is asking about.
///
/// A report written before scopes were recorded has `None` and is only reusable
/// for an unscoped run. That is the safe direction: refusing to reuse costs one
/// sweep, while reusing the wrong one produces a report claiming to have
/// reviewed code it never read.
fn same_scope(report: &LaneReport, scope: Option<&str>) -> bool {
    report.scope.as_deref() == scope
}

/// The model half of a `vendor:model` spec. A report records the resolved
/// `vendor:model`, while a unit may hold a bare alias.
fn model_of(spec: &str) -> &str {
    spec.split_once(':').map_or(spec, |(_, model)| model)
}

pub(super) fn file_name_for(unit: &Unit) -> String {
    // A default unit — no effort, first pass — keeps the original name, so
    // every report written before efforts or passes existed stays resumable.
    if unit.effort.trim().is_empty() && unit.pass <= 1 {
        return format!("{}-{}.json", unit.lane.slug(), safe(&unit.model));
    }
    // Anything else appends BOTH fields, delimited by `~` — a character
    // `safe()` can never emit, so no effort or model text can spell a pass
    // marker and no pass marker can spell an effort. Plain `-` concatenation
    // could: effort `pass2` and a real second pass produced byte-identical
    // names, whichever sweep finished last silently overwrote the other, and
    // resume then handed one unit the other unit's report. Both fields always
    // present (even empty) so the split is positional, not guessed.
    format!(
        "{}-{}~{}~p{}.json",
        unit.lane.slug(),
        safe(&unit.model),
        safe(unit.effort.trim()),
        unit.pass.max(1)
    )
}

/// The name a report would have had under the old, lossy encoding.
///
/// Only ever read, never written. A sweep costs tens of minutes, so a run
/// resumed after this changed should still find what it already paid for —
/// but the old encoding could not tell `codex:a/b` from `codex:a-b`, which is
/// why anything found here is checked against the report's own recorded lane
/// and model before it is believed.
pub(super) fn legacy_file_name_for(unit: &Unit) -> String {
    let lossy = |text: &str| -> String {
        text.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    };
    if unit.effort.trim().is_empty() && unit.pass <= 1 {
        return format!("{}-{}.json", unit.lane.slug(), lossy(&unit.model));
    }
    format!(
        "{}-{}~{}~p{}.json",
        unit.lane.slug(),
        lossy(&unit.model),
        lossy(unit.effort.trim()),
        unit.pass.max(1)
    )
}

/// A filename-safe encoding of one component, from which no two distinct
/// inputs produce the same output.
///
/// Mapping every non-alphanumeric character to a dash was safe but *lossy*:
/// `codex:a/b` and `codex:a-b` both became `codex-a-b`, so two configured
/// models shared one report file. One sweep overwrote the other, and a resumed
/// run handed a model the other model's findings while the merged report
/// stated the wrong provenance.
///
/// Escaping keeps the property that matters — nothing here can be a path
/// separator, a drive letter or `..` — while making the encoding injective:
/// every dash in the output came from a literal dash in the input, and
/// everything else is a hex escape that cannot be produced any other way.
fn safe(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if c == '-' {
            // A literal dash escapes too, or `a-b` and `a/b` would still meet.
            out.push_str("_2d");
        } else {
            // Non-ASCII goes through as its code point, so this stays total.
            out.push_str(&format!("_{:x}", c as u32));
        }
    }
    out
}

/// Write a sweep's report, all of it or none of it.
///
/// Written to a neighbouring temporary file and renamed into place, because
/// `fs::write` truncates first: a process killed mid-write left a half-written
/// report where a complete one had been. Resume already treats an unparseable
/// report as absent, so the cost was not corruption but paying tens of minutes
/// for a sweep twice — and the second time, the first result is gone.
///
/// A rename within one directory is atomic on both platforms this ships to. If
/// the rename fails the temporary file is removed rather than left beside the
/// reports, where its name would not match any unit and it would simply sit
/// there confusing the next reader.
pub(super) fn write_report(dir: &Path, name: &str, report: &LaneReport) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path: PathBuf = dir.join(name);
    let json = serde_json::to_string_pretty(report)?;

    crate::atomic::write(&path, json)
        .map_err(|e| anyhow::anyhow!("cannot replace {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "persist/tests.rs"]
mod tests;
