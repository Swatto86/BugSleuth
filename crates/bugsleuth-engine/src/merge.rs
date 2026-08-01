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

    for path in paths {
        let file = read(path)?;
        match file.status {
            SweepStatus::NotSwept { reason } => unswept.push(Unswept {
                lane: file.lane,
                model: file.model,
                reason,
            }),
            SweepStatus::Swept => {
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
    })
}

fn read(path: &Path) -> Result<SweepFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read sweep report {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} is not a sweep report", path.display()))
}

impl Merged {
    pub fn to_text(&self) -> String {
        let mut out = String::new();

        out.push_str("=== merged report ===\n");
        for source in &self.sources {
            out.push_str(&format!(
                "  swept: {} lane by {} ({} findings)\n",
                source.lane, source.model, source.findings
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
                self.sources.len(),
                models.join(", "),
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, body);
        path
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("bugsleuth-merge-tests")
            .join(format!("{}-{name}", std::process::id()))
    }

    const SWEPT: &str = r#"{
        "lane":"Correctness","model":"claude:sonnet",
        "status":{"state":"swept","turns":8},
        "findings":[{
            "id":"c-0","lane":"correctness","model":"claude:sonnet",
            "title":"average_price divides by zero on an empty inventory",
            "severity":"high",
            "anchor":{"file":"src/inventory.rs","line":43,"claimed_line":43,"snippet":"total / len"},
            "explanation":"No check for an empty inventory before dividing by the item count.",
            "failure_scenario":"f"}],
        "rejected":[]
    }"#;

    const SWEPT_OTHER: &str = r#"{
        "lane":"Correctness","model":"codex:",
        "status":{"state":"swept"},
        "findings":[{
            "id":"x-0","lane":"correctness","model":"codex:",
            "title":"Calculating the average price of an empty inventory panics",
            "severity":"medium",
            "anchor":{"file":"src/inventory.rs","line":43,"claimed_line":43,"snippet":"total / len"},
            "explanation":"An empty inventory has length zero, so this integer division panics.",
            "failure_scenario":"f"}],
        "rejected":[]
    }"#;

    const FAILED: &str = r#"{
        "lane":"Security","model":"codex:",
        "status":{"state":"not_swept","reason":"the codex CLI exited with code 1"},
        "findings":[],"rejected":[]
    }"#;

    #[test]
    fn the_same_defect_from_two_vendors_merges_and_records_agreement() {
        let dir = scratch("merge-two");
        let paths = vec![
            write(&dir, "a.json", SWEPT),
            write(&dir, "b.json", SWEPT_OTHER),
        ];
        let merged = merge(&paths).unwrap_or_else(|e| panic!("merge failed: {e}"));
        assert_eq!(merged.ranked.len(), 1, "the same defect was not merged");
        assert_eq!(merged.ranked[0].cluster.agreement, 2);
        // Severity is normalised upward: one said high, one said medium.
        assert_eq!(
            merged.ranked[0].cluster.severity().as_str(),
            "high",
            "a cluster must not be presented more mildly than its worst assessment"
        );
    }

    #[test]
    fn a_failed_sweep_is_reported_as_unswept_and_never_counted_as_clean() {
        let dir = scratch("merge-failed");
        let paths = vec![write(&dir, "a.json", SWEPT), write(&dir, "f.json", FAILED)];
        let merged = merge(&paths).unwrap_or_else(|e| panic!("merge failed: {e}"));
        assert_eq!(merged.unswept.len(), 1);
        assert_eq!(merged.sources.len(), 1);

        let text = merged.to_text();
        assert!(text.contains("NOT SWEPT"));
        assert!(text.contains("Security"));
        assert!(text.contains("NOT reviewed"));
    }

    #[test]
    fn an_unreadable_report_is_an_error_rather_than_a_silently_empty_merge() {
        let dir = scratch("merge-bad");
        let paths = vec![write(&dir, "bad.json", "{ not json")];
        assert!(merge(&paths).is_err());
    }
}
