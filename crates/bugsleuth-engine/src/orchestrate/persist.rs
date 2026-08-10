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
        .filter(|report| same_scope(report, options.scope) && same_revision(report, options))
        // Reports written before the encoding changed are still worth tens of
        // minutes each, so the old name is tried too — but only after the
        // current one, and only if the report says it is the right sweep. The
        // old encoding was lossy, which is exactly why it is not trusted to
        // identify anything on its own.
        .or_else(|| {
            if unit.use_agents {
                return None;
            }
            // Every grammar, not just the most recent one. The writer changed
            // twice, and probing only the latest predecessor left reports on
            // disk right now — `security-codex_3a.json` among them — invisible,
            // so a resumed run paid for those sweeps a second time.
            historical_file_names_for(unit)
                .into_iter()
                .find_map(|name| {
                    let legacy = read_swept(&dir.join(name))?;
                    // The scope check belongs on this path too. Adding it only
                    // to the branch above would leave the whole defect reachable
                    // through the legacy name, which is exactly the "fixed the
                    // first sink, missed the second" shape the correctness
                    // mandate hunts for.
                    let same_sweep = legacy.lane == unit.lane.title()
                        && legacy.model.ends_with(model_of(&unit.model))
                        && same_scope(&legacy, options.scope)
                        && same_revision(&legacy, options);
                    same_sweep.then_some(legacy)
                })
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

/// Whether a stored report reviewed the exact source revision this run is at.
///
/// A report is only reusable when it recorded a clean revision that still
/// matches the repository's current clean revision. Resume used to key on lane,
/// model and scope alone, so a sweep run at one commit was handed back after the
/// checkout advanced — findings from code that was no longer there, and blind to
/// code that now was. A report with no recorded revision (written before this
/// existed, or taken over a dirty tree) is never reused: swept once more is the
/// safe direction to be wrong in.
fn same_revision(report: &LaneReport, options: &RunOptions<'_>) -> bool {
    let current = crate::sweep::clean_revision(options.repo);
    report.cache_revision.is_some() && report.cache_revision == current
}

/// The model half of a `vendor:model` spec. A report records the resolved
/// `vendor:model`, while a unit may hold a bare alias.
fn model_of(spec: &str) -> &str {
    spec.split_once(':').map_or(spec, |(_, model)| model)
}

pub(super) fn file_name_for(unit: &Unit) -> String {
    // An ordinary default unit — no agents, effort, or repeat pass — keeps the
    // original name, so reports written before those options stay resumable.
    let agents = if unit.use_agents { "~agents" } else { "" };
    if unit.effort.trim().is_empty() && unit.pass <= 1 {
        return format!("{}-{}{}.json", unit.lane.slug(), safe(&unit.model), agents);
    }
    // Anything else appends both positional fields and, when selected, the
    // agent marker. They are delimited by `~` — a character
    // `safe()` can never emit, so no effort or model text can spell a pass
    // marker and no pass marker can spell an effort. Plain `-` concatenation
    // could: effort `pass2` and a real second pass produced byte-identical
    // names, whichever sweep finished last silently overwrote the other, and
    // resume then handed one unit the other unit's report. Both fields always
    // present (even empty) so the split is positional, not guessed.
    format!(
        "{}-{}~{}~p{}{}.json",
        unit.lane.slug(),
        safe(&unit.model),
        safe(unit.effort.trim()),
        unit.pass.max(1),
        agents
    )
}

/// Every name this unit's report could be stored under by a previous release,
/// newest grammar first.
///
/// Each one is a complete read format, not a variation on the current writer:
/// the escaping changed, and before that the delimiters, and before that the
/// escaping did not exist at all. Reconstructing only the most recent
/// predecessor left reports written by the other two — `security-codex_3a.json`
/// is in this repository right now — unfindable, and a resumed run paid for
/// those sweeps a second time.
///
/// Only ever read, never written, and never for an agent-enabled unit: all
/// three predate agent mode. Every candidate is validated against the report's
/// own recorded lane, model, scope and revision before it is believed, because
/// two of the three encodings were lossy — they could not tell `codex:a/b` from
/// `codex:a-b`.
fn historical_file_names_for(unit: &Unit) -> Vec<String> {
    vec![
        prior_escaped_file_name_for(unit),
        legacy_file_name_for(unit),
        original_lossy_file_name_for(unit),
    ]
}

/// Escaping without the terminating `_`, and the positional `~` delimiters.
fn prior_escaped_file_name_for(unit: &Unit) -> String {
    let escaped = |text: &str| -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c);
            } else if c == '-' {
                out.push_str("_2d");
            } else {
                out.push_str(&format!("_{:x}", c as u32));
            }
        }
        out
    };
    if unit.effort.trim().is_empty() && unit.pass <= 1 {
        return format!("{}-{}.json", unit.lane.slug(), escaped(&unit.model));
    }
    format!(
        "{}-{}~{}~p{}.json",
        unit.lane.slug(),
        escaped(&unit.model),
        escaped(unit.effort.trim()),
        unit.pass.max(1)
    )
}

/// Lossy dash-mapping, with the positional `~` delimiters.
pub(super) fn legacy_file_name_for(unit: &Unit) -> String {
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

/// Lossy dash-mapping, before the delimiters: `-<effort>` and `-pass<N>`.
fn original_lossy_file_name_for(unit: &Unit) -> String {
    let effort = if unit.effort.trim().is_empty() {
        String::new()
    } else {
        format!("-{}", lossy(unit.effort.trim()))
    };
    let pass = if unit.pass <= 1 {
        String::new()
    } else {
        format!("-pass{}", unit.pass)
    };
    format!(
        "{}-{}{effort}{pass}.json",
        unit.lane.slug(),
        lossy(&unit.model)
    )
}

/// The encoding both lossy grammars share: every non-alphanumeric is a dash.
fn lossy(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
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
/// every non-alphanumeric character is a self-delimiting hex escape.
fn safe(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if c == '-' {
            // A literal dash escapes too, or `a-b` and `a/b` would still meet.
            out.push_str("_2d_");
        } else {
            // Terminated with `_`, which never appears bare in the output, so
            // an escape cannot be extended by the literal hex digits after it.
            out.push_str(&format!("_{:x}_", c as u32));
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

#[cfg(test)]
#[path = "persist/naming_tests.rs"]
mod naming_tests;
