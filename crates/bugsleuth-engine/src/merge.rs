//! The judge command: merge several sweeps into one ranked list.
//!
//! Provenance is stripped before clustering compares anything — the judge sees
//! wording and anchors, never which vendor said what. Models favour their own
//! family's output, and a merge step that knows the source has a thumb on the
//! scale. Provenance is put back afterwards, because *how many* models agreed is
//! the headline trust signal.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bugsleuth_domain::Finding;
use bugsleuth_judge::{Ranked, cluster, rank};
use serde::Deserialize;

/// The parts of a sweep report the judge needs. Deliberately a separate,
/// narrower type from the one the sweep writes: the judge consumes a file that
/// may have been produced by an older version, and should not fail because a
/// field it never reads was added or renamed.
#[derive(Debug, Deserialize)]
struct SweepFile {
    lane: String,
    model: String,
    /// The commit the sweep reviewed, when its report recorded one.
    #[serde(default)]
    commit: Option<String>,
    status: SweepStatus,
    #[serde(default)]
    findings: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum SweepStatus {
    Swept,
    NotSwept { reason: String },
}

pub struct Merged {
    pub ranked: Vec<Ranked>,
    pub sources: Vec<Source>,
    /// Distinct commits the merged sweeps reviewed, when recorded. More than
    /// one means the report spans two versions of the code — anchors from one
    /// sweep may simply not exist in the tree another reviewed, and anyone
    /// re-checking the findings must be told which tree each came from.
    pub commits: Vec<String>,
    /// Sweeps that did not run. Reported loudly and never silently dropped: a
    /// merged report that quietly omits a failed sweep reads exactly like one
    /// where that sweep found nothing.
    pub unswept: Vec<Unswept>,
}

pub struct Source {
    pub lane: String,
    pub model: String,
    pub findings: usize,
}

pub struct Unswept {
    pub lane: String,
    pub model: String,
    pub reason: String,
}

/// Read sweep reports and merge them.
pub fn merge(paths: &[PathBuf]) -> Result<Merged> {
    let mut all: Vec<Finding> = Vec::new();
    let mut sources = Vec::new();
    let mut unswept = Vec::new();
    let mut commits: Vec<String> = Vec::new();

    for path in paths {
        let file = read(path)?;
        match file.status {
            SweepStatus::NotSwept { reason } => unswept.push(Unswept {
                lane: file.lane,
                model: file.model,
                reason,
            }),
            SweepStatus::Swept => {
                if let Some(commit) = &file.commit
                    && !commits.contains(commit)
                {
                    commits.push(commit.clone());
                }
                sources.push(Source {
                    lane: file.lane,
                    model: file.model,
                    findings: file.findings.len(),
                });
                all.extend(file.findings);
            }
        }
    }

    Ok(Merged {
        ranked: rank(cluster(all)),
        sources,
        unswept,
        commits,
    })
}

fn read(path: &Path) -> Result<SweepFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read sweep report {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} is not a sweep report", path.display()))
}

impl Merged {
    /// How many distinct models could have found a defect in this lane.
    fn models_on(&self, lane: &str) -> usize {
        let mut models: Vec<&str> = self
            .sources
            .iter()
            .filter(|source| source.lane.eq_ignore_ascii_case(lane))
            .map(|source| source.model.as_str())
            .collect();
        models.sort_unstable();
        models.dedup();
        models.len()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();

        out.push_str("=== merged report ===\n");
        for source in &self.sources {
            out.push_str(&format!(
                "  swept: {} lane by {} ({} findings)\n",
                source.lane, source.model, source.findings
            ));
        }
        // Learned the expensive way: a set of correct findings was re-graded
        // against a different checkout and condemned as fabricated, because
        // nothing said which tree they described.
        if self.commits.len() > 1 {
            let short: Vec<&str> = self.commits.iter().map(|c| &c[..c.len().min(9)]).collect();
            out.push_str(&format!(
                "\n  WARNING: these sweeps reviewed {} different commits ({}). The merged\n  \
                 list spans two versions of the code; a finding may cite code that only\n  \
                 exists in the version its own sweep reviewed.\n",
                self.commits.len(),
                short.join(", ")
            ));
        }
        for miss in &self.unswept {
            out.push_str(&format!(
                "  NOT SWEPT: {} lane by {} - {}\n",
                miss.lane, miss.model, miss.reason
            ));
        }
        if !self.unswept.is_empty() {
            out.push_str(
                "  Those combinations were NOT reviewed. Their absence below means\n  \
                 nothing was looked for, not that nothing is there.\n",
            );
        }

        let total: usize = self.sources.iter().map(|s| s.findings).sum();
        out.push_str(&format!(
            "\n  {total} findings from {} sweeps merged into {} distinct defects\n",
            self.sources.len(),
            self.ranked.len()
        ));

        // From the shared module, so a caveat added here cannot go missing
        // from the run report - which is how both of these got out of step.
        out.push_str(&crate::caveats::unsandboxed(
            self.sources.iter().map(|source| &source.model),
            "  ",
        ));
        out.push_str(&crate::caveats::limits("  "));

        for entry in &self.ranked {
            let cluster = &entry.cluster;
            let finding = cluster.representative();
            let models: Vec<String> = cluster.models().iter().map(|m| m.to_string()).collect();
            out.push_str(&format!(
                "\n  {}. [{}] {}\n     {}:{}\n     found by {} of {} models: {}\n",
                entry.position,
                cluster.severity().as_str().to_uppercase(),
                finding.title,
                finding.anchor.file,
                finding.anchor.line,
                cluster.agreement,
                // Distinct models that swept this defect's lane, not the number
                // of sweeps: two models across three sweeps printed "1 of 3
                // models", understating agreement against a total that never
                // existed.
                self.models_on(finding.lane.as_str()),
                models.join(", "),
            ));
        }

        if !self.ranked.is_empty() {
            out.push_str(
                "
  A prompt for fixing these — one work order per defect, written
                   for a coding agent — is available separately.
",
            );
        }
        out
    }
}

impl Merged {
    /// The whole thing as one prompt, to paste into a coding agent.
    ///
    /// Deliberately not part of `to_text`. The report is for a person deciding
    /// what matters; this is for a model doing the work, and mixing them made a
    /// document that served neither — the reader scrolled past pages of
    /// replacement code, and anyone copying it for an agent sent along a summary
    /// the agent had no use for.
    ///
    /// Includes the unswept lanes on purpose. An agent handed a list of defects
    /// should know which parts of the repository nobody looked at, or it will
    /// reasonably assume the list is complete.
    #[must_use]
    pub fn to_fix_prompt(&self, repo: &str) -> String {
        let skipped: Vec<String> = self
            .unswept
            .iter()
            .map(|m| format!("{} lane, by {} — {}", m.lane, m.model, m.reason))
            .collect();
        crate::handoff::prompt(repo, &self.ranked, &skipped, self.sources.len())
    }
}

#[cfg(test)]
#[path = "merge/tests.rs"]
mod tests;
