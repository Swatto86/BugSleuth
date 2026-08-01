//! Rendering a sweep's outcome.
//!
//! Two audiences. The JSON form is for the eval harness and, later, the app. The
//! text form is for a human, and it has one unusual requirement: it must never
//! let "this lane found nothing" be confused with "this lane never ran". A clean
//! report that silently omitted an unswept lane is the most dangerous output
//! this tool could produce, so absence is always stated explicitly.

use bugsleuth_domain::{Finding, Severity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LaneReport {
    pub lane: String,
    pub model: String,
    pub status: Status,
    pub findings: Vec<Finding>,
    /// Findings the model reported that failed anchor verification, with the
    /// reason. Kept rather than dropped silently: the rate is the headline
    /// measure of whether a model can be trusted.
    pub rejected: Vec<Rejected>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    /// The lane ran to completion.
    Swept {
        #[serde(default)]
        turns: Option<u32>,
    },
    /// The lane did not run, or did not finish. Never rendered as "no findings".
    NotSwept { reason: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Rejected {
    pub title: String,
    pub claimed_file: String,
    pub claimed_line: u32,
    pub reason: String,
}

impl LaneReport {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("=== {} lane · {} ===\n", self.lane, self.model));

        match &self.status {
            Status::NotSwept { reason } => {
                out.push_str(&format!(
                    "  NOT SWEPT — {reason}\n  This lane was NOT reviewed. Its absence from the\n  findings below means nothing was looked for, not that nothing is there.\n"
                ));
                return out;
            }
            Status::Swept { turns } => {
                let turns = turns.map(|t| format!(" in {t} turns")).unwrap_or_default();
                out.push_str(&format!(
                    "  swept{turns} · {} verified, {} discarded\n",
                    self.findings.len(),
                    self.rejected.len()
                ));
            }
        }

        if self.findings.is_empty() {
            out.push_str("  No verified findings.\n");
        }
        for finding in &self.findings {
            out.push_str(&format!(
                "\n  [{}] {}\n    {}:{}\n",
                finding.severity.as_str().to_uppercase(),
                finding.title,
                finding.anchor.file,
                finding.anchor.line,
            ));
            if finding.anchor.was_corrected() {
                out.push_str(&format!(
                    "    (model said line {}; corrected)\n",
                    finding.anchor.claimed_line
                ));
            }
            for line in finding.anchor.snippet.lines() {
                out.push_str(&format!("    | {line}\n"));
            }
            out.push_str(&format!("    Why:  {}\n", finding.explanation));
            out.push_str(&format!("    When: {}\n", finding.failure_scenario));
        }

        if !self.rejected.is_empty() {
            out.push_str("\n  Discarded (quoted code not found in the file named):\n");
            for rejected in &self.rejected {
                out.push_str(&format!(
                    "    - {} [{}:{}] — {}\n",
                    rejected.title, rejected.claimed_file, rejected.claimed_line, rejected.reason
                ));
            }
        }
        out
    }
}

/// Most severe first, then by file so the order is stable across runs.
pub fn rank(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        severity_rank(a.severity)
            .cmp(&severity_rank(b.severity))
            .then_with(|| a.anchor.file.cmp(&b.anchor.file))
            .then_with(|| a.anchor.line.cmp(&b.anchor.line))
    });
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Critical => 0,
        Severity::High => 1,
        Severity::Medium => 2,
        Severity::Low => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::{FindingId, LaneId, ModelId, VerifiedAnchor};

    fn finding(severity: Severity, file: &str, line: u32) -> Finding {
        Finding {
            id: FindingId::new("f1"),
            lane: LaneId::new("correctness"),
            model: ModelId::new("claude:sonnet"),
            title: "t".into(),
            severity,
            anchor: VerifiedAnchor {
                file: file.into(),
                line,
                claimed_line: line,
                snippet: "code".into(),
            },
            explanation: "e".into(),
            failure_scenario: "f".into(),
        }
    }

    #[test]
    fn an_unswept_lane_is_never_rendered_as_having_no_findings() {
        let report = LaneReport {
            lane: "Security".into(),
            model: "claude:sonnet".into(),
            status: Status::NotSwept {
                reason: "no model assigned".into(),
            },
            findings: vec![],
            rejected: vec![],
        };
        let text = report.to_text();
        assert!(text.contains("NOT SWEPT"));
        assert!(!text.contains("No verified findings"));
    }

    #[test]
    fn a_swept_lane_with_nothing_found_says_so_explicitly() {
        let report = LaneReport {
            lane: "Security".into(),
            model: "claude:sonnet".into(),
            status: Status::Swept { turns: Some(4) },
            findings: vec![],
            rejected: vec![],
        };
        assert!(report.to_text().contains("No verified findings"));
    }

    #[test]
    fn findings_rank_most_severe_first() {
        let mut findings = vec![
            finding(Severity::Low, "a.rs", 1),
            finding(Severity::Critical, "z.rs", 9),
            finding(Severity::Medium, "b.rs", 2),
        ];
        rank(&mut findings);
        let order: Vec<Severity> = findings.iter().map(|f| f.severity).collect();
        assert_eq!(
            order,
            vec![Severity::Critical, Severity::Medium, Severity::Low]
        );
    }
}
