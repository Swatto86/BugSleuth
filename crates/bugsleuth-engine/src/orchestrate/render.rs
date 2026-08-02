//! Rendering a run's outcome.
//!
//! The rules this file enforces are all about not overstating what happened: a
//! hole is always named, a lane that found nothing is distinguishable from one
//! that never ran, and severities are not presented as comparable across lanes
//! when they are not.

use bugsleuth_domain::Lane;

use super::RunReport;

impl RunReport {
    /// How many distinct lanes actually ran.
    fn lanes_swept(&self) -> usize {
        let mut lanes: Vec<Lane> = self.swept.iter().map(|sweep| sweep.lane).collect();
        lanes.sort_by_key(|lane| lane.slug());
        lanes.dedup();
        lanes.len()
    }

    /// How many distinct models could have found a defect in this lane.
    ///
    /// The denominator used to be the number of *sweeps*, which is a different
    /// number the moment a model covers two lanes or repeats a pass: two models
    /// across three sweeps printed "found by 1 of 3 models", understating
    /// agreement against a total that never existed. Only models that swept the
    /// defect's own lane could have found it, so only they belong in the count.
    fn models_on(&self, lane: &str) -> usize {
        let mut models: Vec<&str> = self
            .swept
            .iter()
            .filter(|sweep| sweep.lane.slug() == lane)
            .map(|sweep| sweep.model.as_str())
            .collect();
        models.sort_unstable();
        models.dedup();
        models.len()
    }

    /// Whether one grader compared every defect in this report against every
    /// other. A partial pass does not count: half a list graded one way and
    /// half another is exactly the state the cross-lane warning is for.
    fn graded_together(&self) -> bool {
        self.triage.note.is_empty()
            && self.triage.graded > 0
            && self.triage.graded == self.ranked.len()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("=== run report ===\n");
        for sweep in &self.swept {
            // The rejection count is trust information, not noise: it says how
            // often this model's claims about this repository failed the only
            // mechanical check applied to them.
            let rejected = if sweep.rejected == 0 {
                String::new()
            } else {
                format!(", {} rejected as unverifiable", sweep.rejected)
            };
            // A salvaged sweep is recovered work, not a clean run. Rendering
            // the two identically would let a review that was cut off partway
            // read as a thorough one that simply found little.
            let salvaged = if sweep.salvaged {
                " — RECOVERED after running out of turns, so this list is as far as it got"
            } else {
                ""
            };
            out.push_str(&format!(
                "  swept: {} lane by {} ({} findings{rejected}){salvaged}\n",
                sweep.lane.title(),
                sweep.model,
                sweep.findings,
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

        let total: usize = self.swept.iter().map(|sweep| sweep.findings).sum();
        out.push_str(&format!(
            "\n  {total} findings from {} sweeps merged into {} distinct defects\n",
            self.swept.len(),
            self.ranked.len()
        ));

        // What ordered this list, in one line. The reader cannot check the code,
        // so "worst first" has to say whose judgement that is.
        if !self.triage.note.is_empty() {
            out.push_str(&format!("\n  Note: {}.\n", self.triage.note));
        } else if self.triage.graded > 0 {
            out.push_str(&format!(
                "\n  Severities were re-graded across the whole list, with every defect\n  \
                 in view ({} of {} moved from what the model that found it said).\n",
                self.triage.changed, self.triage.graded
            ));
        }

        // Severity means different things in different mandates. A "high" from
        // the security lane and a "high" from the correctness lane were assigned
        // by models answering different questions, so ordering one against the
        // other is not a judgement anyone actually made. Say so rather than let
        // a single ranked list imply a comparison it cannot support.
        //
        // Unless a triage pass graded the whole list, in which case that
        // comparison *was* made, by one grader against one rubric. Repeating the
        // warning then would be telling the reader not to trust the one thing
        // that was done specifically to make the ordering trustworthy.
        if self.lanes_swept() > 1 && !self.graded_together() {
            out.push_str(
                "\n  Note: this list spans more than one lane. Severities are relative to\n  \
                 each lane's own mandate and are NOT directly comparable between lanes -\n  \
                 read the ordering within a lane, not across them.\n",
            );
        }

        // Both caveats come from one place. Each was once added to a single
        // renderer and forgotten in the other, so the same report read more
        // reassuringly depending only on which command produced it.
        out.push_str(&crate::caveats::unsandboxed(
            self.swept.iter().map(|sweep| &sweep.model),
            "  ",
        ));
        out.push_str(&crate::caveats::limits("  "));

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
                self.models_on(finding.lane.as_str()),
            ));
            // A moved grade is shown with what it moved from and the stated
            // consequence that decided it. Silently replacing a model's own
            // assessment would hide the one judgement about this defect that no
            // reviewer of the code actually made.
            if let Some(graded) = cluster.triaged
                && graded != cluster.claimed_severity()
            {
                let why = cluster
                    .triage_reason
                    .as_deref()
                    .map(|reason| format!(" — {reason}"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "     re-graded: the model that found it said {}{why}\n",
                    cluster.claimed_severity().as_str(),
                ));
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "render/tests.rs"]
mod tests;
