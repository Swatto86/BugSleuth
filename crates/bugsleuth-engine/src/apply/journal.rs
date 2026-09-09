//! What a fix run has already done, on disk, so an interruption is not a loss.
//!
//! The thing being protected is not cheap. Each entry here represents a defect
//! a model has already read, fixed, tested and committed — minutes of work and
//! real subscription quota. The usual way a run ends early is that the quota ran
//! out, which is also the case where starting over is most expensive, so the
//! record is written after every single defect rather than at the end.
//!
//! It records what cannot be recovered by looking at the repository afterwards.
//! Git shows *that* commits exist; it does not say which defect each one was
//! for, and matching them back by message is guesswork that would either redo a
//! defect or skip a half-finished one.
//!
//! A journal is only ever used to continue the run it was written for. If the
//! report has been re-swept, or the fixing model changed, the work orders are
//! not the same work orders and the journal is discarded rather than applied to
//! them — see [`contract`].

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::Baseline;
use super::steps::Step;

/// The file, beside the prompts it tracks.
const FILE: &str = "apply-progress.json";

#[derive(Serialize, Deserialize)]
struct State {
    /// Identifies the work orders this journal belongs to. A mismatch means the
    /// report was re-run or the model changed, and the journal is not about
    /// this apply at all.
    contract: String,
    /// Where the repository stood before the *first* defect was fixed.
    ///
    /// Carried rather than re-read, and this is the whole reason the file needs
    /// to exist beyond a list of positions: on resume, everything measured
    /// against the baseline — what changed, what to strip attribution from,
    /// what to push — has to cover the commits the earlier attempt made too. A
    /// baseline sampled at resume would silently exclude them, and the fixes
    /// from the first attempt would be pushed with an AI trailer still on them.
    baseline: Baseline,
    /// Defects finished, in the order they were finished.
    done: Vec<Done>,
}

#[derive(Serialize, Deserialize)]
struct Done {
    position: usize,
    /// The model's own account of that defect. Kept because the finished report
    /// quotes it, and after a resume the earlier attempt's account is otherwise
    /// gone — the run would describe only the defects fixed since the failure.
    text: String,
}

/// A fix run's progress, loaded or started fresh.
pub(crate) struct Journal {
    path: PathBuf,
    state: State,
}

impl Journal {
    /// Continue the journal for these work orders, or begin one.
    ///
    /// `baseline` is only used when beginning: a journal that already exists
    /// owns the baseline, for the reason given on the field.
    pub(crate) fn open(dir: &Path, model: &str, steps: &[Step], baseline: Baseline) -> Self {
        let path = dir.join(FILE);
        let contract = contract(model, steps);
        let state = read(&path)
            .filter(|state| state.contract == contract)
            .unwrap_or(State {
                contract,
                baseline,
                done: Vec::new(),
            });
        Self { path, state }
    }

    /// Where the repository stood before the first defect of this run.
    pub(crate) fn baseline(&self) -> &Baseline {
        &self.state.baseline
    }

    /// Whether this defect was already fixed, by this attempt or an earlier one.
    pub(crate) fn is_done(&self, position: usize) -> bool {
        self.state.done.iter().any(|done| done.position == position)
    }

    /// How many defects are already fixed.
    pub(crate) fn completed(&self) -> usize {
        self.state.done.len()
    }

    /// Record a finished defect and put it beyond the reach of an interruption.
    ///
    /// A write failure is returned rather than swallowed. Continuing would run
    /// the remaining defects with no way to record them either, so the run would
    /// present itself as resumable while keeping no evidence — and the next
    /// attempt would redo everything this one paid for.
    ///
    /// # Errors
    /// If the journal cannot be written.
    pub(crate) fn record(&mut self, position: usize, text: &str) -> Result<()> {
        if !self.is_done(position) {
            self.state.done.push(Done {
                position,
                text: text.to_string(),
            });
        }
        let json = serde_json::to_vec_pretty(&self.state)
            .context("cannot describe the fix run's progress")?;
        crate::atomic::write(&self.path, json).with_context(|| {
            format!(
                "the fix was committed, but its progress could not be recorded at {}",
                self.path.display()
            )
        })
    }

    /// Every account the model gave, earlier attempts included, oldest first.
    pub(crate) fn text(&self) -> String {
        self.state
            .done
            .iter()
            .map(|done| done.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Forget this run: it finished, so there is nothing left to resume.
    ///
    /// A removal failure is deliberately not an error. The apply succeeded and
    /// has been reported; the worst a leftover journal does is offer a resume
    /// that finds every defect already done and reaches the same end.
    pub(crate) fn discard(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// What a journal on disk says, if it is readable and parses.
///
/// An unreadable or malformed file is treated as absent. The likeliest cause is
/// a process killed mid-write, and the right answer to that is to fix the
/// defects again — expensive, but correct — rather than to refuse to apply
/// anything until a user deletes a file they were never told about.
fn read(path: &Path) -> Option<State> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Identity of a set of work orders under one model.
///
/// Over the prompts themselves, not their positions: a re-swept report can put
/// a different defect at position 3, and resuming past it would leave that
/// defect unfixed while reporting it done. The model is included because the
/// same defect handed to a different model is a different piece of work — the
/// user changed it for a reason, and the defects already fixed by the old one
/// are the ones they are least likely to want kept.
fn contract(model: &str, steps: &[Step]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    let mut include = |text: &str| {
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    };
    include(model);
    for step in steps {
        include(&step.position.to_string());
        include(&step.prompt);
    }
    format!("{hash:016x}")
}

/// How far an unfinished fix run for this repository got, if there is one
/// that `model` would actually continue.
///
/// Read by the desktop shell so a relaunched window can offer to resume rather
/// than silently starting the whole report again. Deliberately says nothing
/// about *which* defects: the numbers are what a person needs to decide, and
/// the engine re-reads the journal itself when the resume actually happens.
///
/// Judged under the same [`contract`] a resume would use. A journal left by a
/// report that has since been re-swept, or written under a different fixing
/// model, is one the engine will discard and start over — and a button that
/// says "Resume (2 done)" over that is promising work the run will not skip.
#[must_use]
pub fn unfinished(dir: &Path, model: &str) -> Option<usize> {
    let state = read(&dir.join(FILE))?;
    let steps = super::steps::load(dir).ok()?;
    if state.contract != contract(model, &steps) {
        return None;
    }
    let done = state.done.len();
    (done > 0).then_some(done)
}

#[cfg(test)]
#[path = "journal/tests.rs"]
mod tests;
