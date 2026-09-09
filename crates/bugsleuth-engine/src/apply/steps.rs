//! The work orders a fix run executes, and the order it executes them in.
//!
//! A fix run used to be one conversation holding the whole report. That is the
//! wrong shape for something that can be interrupted: a run killed at the third
//! defect of eight had committed three real fixes and recorded nothing about
//! which three, so the only way to continue was to hand the model the whole
//! list again and hope it noticed what was already done.
//!
//! So the unit of work is one defect. `crate::handoff` already writes each as a
//! self-contained prompt beside the bundle, precisely so a defect can be worked
//! on alone — this reads those back. The cost is real and worth naming: the
//! model no longer sees the whole list at once, so it cannot notice that two
//! defects share one cause. What it buys is that every completed defect stays
//! completed.

use std::path::Path;

use anyhow::{Context, Result};

/// One defect's self-contained fix prompt.
#[derive(Debug)]
pub(crate) struct Step {
    /// The defect's position in the ranked report, taken from the file name.
    ///
    /// Not an index into anything: acknowledged findings are not fix work and
    /// get no prompt, so positions skip. It is an identity, and it is what the
    /// journal records, so the numbering the user sees in the report is the
    /// numbering that resume talks about.
    pub position: usize,
    pub prompt: String,
}

/// The whole-report bundle, used when there are no per-defect prompts.
const BUNDLE: &str = "fix-prompt.md";

/// Every defect prompt in `dir`, lowest position first.
///
/// Falls back to the bundle as a single step. A run directory written before
/// per-defect prompts existed has only the bundle, and refusing to apply it
/// would turn an old but perfectly good report into an error; it simply cannot
/// be resumed part-way, which is what it already was.
///
/// # Errors
/// When the directory cannot be listed, or when neither a defect prompt nor a
/// bundle is there to read — an apply with nothing to do must say so rather
/// than report a successful run that changed nothing.
pub(crate) fn load(dir: &Path) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("cannot list the fix prompts in {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot read an entry in {}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Some(position) = defect_position(&name) else {
            continue;
        };
        let prompt = std::fs::read_to_string(entry.path())
            .with_context(|| format!("cannot read the fix prompt {name}"))?;
        steps.push(Step { position, prompt });
    }
    steps.sort_by_key(|step| step.position);

    if steps.is_empty() {
        let bundle = dir.join(BUNDLE);
        let prompt = std::fs::read_to_string(&bundle).with_context(|| {
            format!(
                "no fix prompt for this repository at {}. Run a review first.",
                bundle.display()
            )
        })?;
        // Position 0 is the bundle's own identity, distinct from any defect's.
        steps.push(Step {
            position: 0,
            prompt,
        });
    }
    Ok(steps)
}

/// The defect number in a `fix-prompt-NN.md` name, if that is what this is.
///
/// Requiring the `.md` ending is also what excludes a prompt currently being
/// written: `crate::atomic` stages under a `.writing` suffix, so a half-written
/// work order cannot match here and cannot be handed to a model as a defect.
fn defect_position(name: &str) -> Option<usize> {
    name.strip_prefix("fix-prompt-")?
        .strip_suffix(".md")?
        .parse()
        .ok()
}

#[cfg(test)]
#[path = "steps/tests.rs"]
mod tests;
