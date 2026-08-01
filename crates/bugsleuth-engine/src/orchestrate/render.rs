//! Rendering a run's outcome.
//!
//! The rules this file enforces are all about not overstating what happened: a
//! hole is always named, a lane that found nothing is distinguishable from one
//! that never ran, and severities are not presented as comparable across lanes
//! when they are not.

use bugsleuth_domain::Lane;

use super::{RunEvent, RunReport};

impl RunReport {
    /// How many distinct lanes actually ran.
    fn lanes_swept(&self) -> usize {
        let mut lanes: Vec<Lane> = self.swept.iter().map(|(_, lane, _)| *lane).collect();
        lanes.sort_by_key(|lane| lane.slug());
        lanes.dedup();
        lanes.len()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("=== run report ===\n");
        for (model, lane, count) in &self.swept {
            out.push_str(&format!(
                "  swept: {} lane by {model} ({count} findings)\n",
                lane.title()
            ));
        }
        for gap in &self.gaps {
            let who = gap.model.as_deref().unwrap_or("(nobody)");
            out.push_str(&format!(
                "  NOT SWEPT: {} lane by {who} - {}\n",
                gap.lane.title(),
                gap.reason
            ));
        }
        if !self.gaps.is_empty() {
            out.push_str(
                "\n  The lanes above were NOT reviewed. Their absence from the findings\n  \
                 below means nothing was looked for, not that nothing is there.\n",
            );
        }

        let total: usize = self.swept.iter().map(|(_, _, n)| n).sum();
        out.push_str(&format!(
            "\n  {total} findings from {} sweeps merged into {} distinct defects\n",
            self.swept.len(),
            self.ranked.len()
        ));

        // Severity means different things in different mandates. A "high" from
        // the security lane and a "high" from the correctness lane were assigned
        // by models answering different questions, so ordering one against the
        // other is not a judgement anyone actually made. Say so rather than let
        // a single ranked list imply a comparison it cannot support.
        if self.lanes_swept() > 1 {
            out.push_str(
                "\n  Note: this list spans more than one lane. Severities are relative to\n  \
                 each lane's own mandate and are NOT directly comparable between lanes -\n  \
                 read the ordering within a lane, not across them.\n",
            );
        }

        for entry in &self.ranked {
            let cluster = &entry.cluster;
            let finding = cluster.representative();
            out.push_str(&format!(
                "\n  {}. [{}] {}\n     {}:{}\n     found by {} of {} models\n",
                entry.position,
                cluster.severity().as_str().to_uppercase(),
                finding.title,
                finding.anchor.file,
                finding.anchor.line,
                cluster.agreement,
                self.swept.len(),
            ));
        }
        out
    }
}

/// One line describing a run event, for a terminal.
pub fn describe(event: &RunEvent) -> String {
    match event {
        RunEvent::BatchStarted {
            index,
            total,
            units,
        } => format!("round {index}/{total}: {}", units.join(", ")),
        RunEvent::Reused { model, lane } => {
            format!("reusing {model} x {lane} from a previous run")
        }
        RunEvent::SweepFinished {
            model,
            lane,
            findings,
            swept: true,
            ..
        } => format!("  {model} x {lane}: {findings} findings"),
        RunEvent::SweepFinished {
            model,
            lane,
            reason,
            ..
        } => format!("  {model} x {lane}: NOT SWEPT - {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrate::Gap;

    fn report(gaps: Vec<Gap>) -> RunReport {
        RunReport {
            ranked: vec![],
            swept: vec![("claude:sonnet".into(), Lane::Correctness, 0)],
            gaps,
        }
    }

    #[test]
    fn a_lane_with_no_model_is_named_in_the_report() {
        let text = report(vec![Gap {
            lane: Lane::Security,
            model: None,
            reason: "no model is assigned to this lane".into(),
        }])
        .to_text();
        assert!(text.contains("NOT SWEPT"));
        assert!(text.contains("Security"));
        assert!(text.contains("NOT reviewed"));
    }

    #[test]
    fn a_failed_sweep_is_named_with_its_reason() {
        let text = report(vec![Gap {
            lane: Lane::Ux,
            model: Some("kilo:".into()),
            reason: "the kilo CLI exited with code 1".into(),
        }])
        .to_text();
        assert!(text.contains("NOT SWEPT"));
        assert!(text.contains("kilo:"));
        assert!(text.contains("exited with code 1"));
    }

    #[test]
    fn a_single_lane_report_does_not_warn_about_comparing_lanes() {
        assert!(!report(vec![]).to_text().contains("NOT directly comparable"));
    }

    #[test]
    fn a_multi_lane_report_warns_that_severities_are_not_comparable() {
        // A "high" from the security lane and a "high" from the correctness lane
        // were assigned by models answering different questions.
        let multi = RunReport {
            ranked: vec![],
            swept: vec![
                ("claude:sonnet".into(), Lane::Correctness, 1),
                ("claude:sonnet".into(), Lane::Security, 1),
            ],
            gaps: vec![],
        };
        assert_eq!(multi.lanes_swept(), 2);
        assert!(multi.to_text().contains("NOT directly comparable"));
    }

    #[test]
    fn two_models_on_one_lane_is_still_one_lane() {
        let same_lane = RunReport {
            ranked: vec![],
            swept: vec![
                ("claude:sonnet".into(), Lane::Correctness, 1),
                ("codex:".into(), Lane::Correctness, 1),
            ],
            gaps: vec![],
        };
        assert_eq!(same_lane.lanes_swept(), 1);
        assert!(!same_lane.to_text().contains("NOT directly comparable"));
    }

    #[test]
    fn a_complete_run_says_nothing_about_unswept_lanes() {
        let text = report(vec![]).to_text();
        assert!(!text.contains("NOT SWEPT"));
        assert!(!text.contains("NOT reviewed"));
    }

    #[test]
    fn a_swept_lane_that_found_nothing_is_not_confused_with_a_gap() {
        let text = report(vec![]).to_text();
        assert!(text.contains("swept: Correctness lane"));
        assert!(text.contains("0 findings"));
    }
}
