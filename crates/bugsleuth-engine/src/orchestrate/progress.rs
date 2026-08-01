//! One line per run event, for a terminal or a log pane.
//!
//! Separate from the report because they answer different questions: this says
//! what is happening right now, and the report says what was found. They were
//! together while both were three lines each.

use super::RunEvent;

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

    #[test]
    fn a_failed_sweep_reads_as_a_failure_rather_than_as_zero_findings() {
        // "0 findings" and "did not run" are the two states this tool exists to
        // keep apart, and they are one word apart in a progress log.
        let failed = describe(&RunEvent::SweepFinished {
            model: "kilo:".into(),
            lane: "UX".into(),
            findings: 0,
            swept: false,
            reason: "rate limited".into(),
        });
        assert!(failed.contains("NOT SWEPT"), "{failed}");
        assert!(failed.contains("rate limited"), "{failed}");

        let clean = describe(&RunEvent::SweepFinished {
            model: "kilo:".into(),
            lane: "UX".into(),
            findings: 0,
            swept: true,
            reason: String::new(),
        });
        assert!(!clean.contains("NOT SWEPT"), "{clean}");
        assert!(clean.contains("0 findings"), "{clean}");
    }
}
