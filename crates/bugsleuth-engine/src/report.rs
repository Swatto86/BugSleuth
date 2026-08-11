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
    /// The commit the reviewed tree was at, when the repository is git-backed.
    ///
    /// Exists because of a real mistake it would have prevented: a set of
    /// findings was later re-graded against a *different checkout* of the same
    /// project, and three correct findings were condemned as fabricated because
    /// the code they cited genuinely was not there. A report that says what it
    /// reviewed cannot be mis-graded that way — and `#[serde(default)]` keeps
    /// every report written before this existed loadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The source revision this sweep may be reused for, set only when the
    /// repository was clean and unchanged across the whole sweep.
    ///
    /// Resume reused a stored sweep by lane, model and scope alone, so a run at
    /// a later commit was handed findings from an earlier one and presented them
    /// as a review of the current tree — a report claiming to have read code it
    /// never saw. A sweep that read a dirty tree, or whose HEAD moved under it,
    /// leaves this `None` and is never reused. `#[serde(default)]` keeps reports
    /// written before this existed loadable; they are simply swept again once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_revision: Option<String>,
    /// The path scope the sweep was pointed at, when one was given.
    ///
    /// Recorded because resume identified a reusable sweep by lane and model
    /// alone. Narrowing or changing the scope and running again reused a report
    /// that had reviewed *different code*, and presented it as a review of the
    /// new scope — the tool's one unforgivable output, a lane that says it
    /// looked at something it never read. `#[serde(default)]` keeps reports
    /// written before this existed loadable; they are simply not reusable
    /// under a scope, which is the safe direction to be wrong in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub status: Status,
    pub findings: Vec<Finding>,
    /// Findings the model reported that failed anchor verification, with the
    /// reason. Kept rather than dropped silently: the rate is the headline
    /// measure of whether a model can be trusted.
    pub rejected: Vec<Rejected>,
    /// Token accounting from the provider, when it reports it. Surfaced in the
    /// report so a sweep that cost real quota cannot be read as having spent
    /// nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    /// The lane ran to completion.
    Swept {
        #[serde(default)]
        turns: Option<u32>,
        /// True when the answer was recovered from a session that ran out of
        /// turns mid-review. Recovered work is worth far more than a lost
        /// lane, but it is not a clean sweep: the reviewer was cut off and
        /// asked afterwards what it had, so a short list here means "as far as
        /// it got", not "that is all there is".
        #[serde(default)]
        salvaged: bool,
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
            // `salvaged` bound, not swallowed by `..`. It was, and this report
            // presented a sweep that ran out of turns as a finished one — the
            // reader was told a prefix of a lane's findings was the whole of it.
            Status::Swept { turns, salvaged } => {
                let turns = turns.map(|t| format!(" in {t} turns")).unwrap_or_default();
                out.push_str(&format!(
                    "  swept{turns} · {} verified, {} discarded{}\n",
                    self.findings.len(),
                    self.rejected.len(),
                    crate::caveats::salvaged(*salvaged)
                ));
                if let Some(usage) = &self.usage {
                    out.push_str(&format!("  usage: {usage}\n"));
                }
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
            // Made printable first. The snippet is text from the repository
            // under review, which this tool treats as hostile by design, and an
            // escape sequence inside it is acted on by the terminal rather than
            // shown — it can recolour, clear, or overwrite the lines above it,
            // which in a list of security findings means a repository hiding
            // the finding about itself.
            for line in bugsleuth_domain::printable(&finding.anchor.snippet).lines() {
                out.push_str(&format!("    | {line}\n"));
            }
            out.push_str(&format!("    Why:  {}\n", finding.explanation));
            out.push_str(&format!("    When: {}\n", finding.failure_scenario));
        }

        if !self.rejected.is_empty() {
            out.push_str("\n  Discarded (quoted code not found in the file named):\n");
            for rejected in &self.rejected {
                // Sanitised at the terminal boundary, exactly as verified
                // findings are. These values are model-controlled and never
                // passed through `printable` at construction, so an ANSI or
                // cursor-control sequence in a rejected title or reason would
                // be acted on by the terminal and could hide the finding.
                let title = bugsleuth_domain::printable(&rejected.title);
                let claimed_file = bugsleuth_domain::printable(&rejected.claimed_file);
                let reason = bugsleuth_domain::printable(&rejected.reason);
                out.push_str(&format!(
                    "    - {title} [{claimed_file}:{}] — {reason}\n",
                    rejected.claimed_line
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
            fix: Default::default(),
        }
    }

    #[test]
    fn an_unswept_lane_is_never_rendered_as_having_no_findings() {
        let report = LaneReport {
            lane: "Security".into(),
            model: "claude:sonnet".into(),
            commit: None,
            cache_revision: None,
            scope: None,
            status: Status::NotSwept {
                reason: "no model assigned".into(),
            },
            findings: vec![],
            rejected: vec![],
            usage: None,
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
            commit: None,
            cache_revision: None,
            scope: None,
            status: Status::Swept {
                turns: Some(4),
                salvaged: false,
            },
            findings: vec![],
            rejected: vec![],
            usage: None,
        };
        assert!(report.to_text().contains("No verified findings"));
    }

    #[test]
    fn a_swept_lane_reports_usage_when_it_is_known() {
        let report = LaneReport {
            lane: "Security".into(),
            model: "claude:sonnet".into(),
            commit: None,
            cache_revision: None,
            scope: None,
            status: Status::Swept {
                turns: Some(4),
                salvaged: false,
            },
            findings: vec![],
            rejected: vec![],
            usage: Some("input_tokens=50000, output_tokens=5000, cache_creation_input_tokens=1000, cache_read_input_tokens=2000".into()),
        };
        let text = report.to_text();
        assert!(text.contains("usage:"), "{text}");
        assert!(text.contains("input_tokens=50000"), "{text}");
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

    /// A rejected finding's model-controlled fields are terminal output, so they
    /// must be sanitised like a verified finding's snippet is. Without this, a
    /// rejected title or reason carrying an ANSI clear-screen sequence would be
    /// acted on by the terminal and could hide the report's findings.
    #[test]
    fn rejected_finding_fields_are_stripped_of_terminal_control_characters() {
        let report = LaneReport {
            lane: "Security".into(),
            model: "claude:sonnet".into(),
            commit: None,
            cache_revision: None,
            scope: None,
            status: Status::Swept {
                turns: None,
                salvaged: false,
            },
            findings: vec![],
            rejected: vec![Rejected {
                title: "\u{1b}[2Jhidden".into(),
                claimed_file: "\u{1b}[2Jsrc/x.rs".into(),
                claimed_line: 4,
                reason: "\u{1b}[2Jnot there".into(),
            }],
            usage: None,
        };
        let text = report.to_text();
        assert!(
            text.contains("<0x1b>"),
            "the escape was dropped entirely: {text}"
        );
        assert!(
            !text.contains('\u{1b}'),
            "a raw control character reached the terminal output: {text}"
        );
    }
}

#[cfg(test)]
mod salvaged_tests {
    use super::*;

    fn swept(salvaged: bool) -> LaneReport {
        LaneReport {
            lane: "Correctness".into(),
            model: "claude:sonnet".into(),
            commit: None,
            cache_revision: None,
            scope: None,
            status: Status::Swept {
                turns: Some(30),
                salvaged,
            },
            findings: vec![],
            rejected: vec![],
            usage: None,
        }
    }

    /// The standalone `sweep` report dropped this with a `..` pattern, so a
    /// sweep that ran out of turns read as a finished one and its partial list
    /// of findings read as the whole of what is there.
    #[test]
    fn a_recovered_sweep_says_so_in_the_standalone_report() {
        let text = swept(true).to_text();
        assert!(text.contains("RECOVERED"), "{text}");
    }

    #[test]
    fn a_finished_sweep_is_not_labelled_recovered() {
        let text = swept(false).to_text();
        assert!(!text.contains("RECOVERED"), "{text}");
    }
}
