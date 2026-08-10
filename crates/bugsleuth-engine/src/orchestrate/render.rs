//! Rendering a run's outcome.
//!
//! The rules this file enforces are all about not overstating what happened: a
//! hole is always named, a lane that found nothing is distinguishable from one
//! that never ran, and severities are not presented as comparable across lanes
//! when they are not.

use bugsleuth_domain::{Lane, Severity};

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

    /// Whose judgement put this list in this order.
    ///
    /// The reader cannot check the code, so "worst first" has to say who
    /// decided it — and a pass that did not run must say that rather than let
    /// silence read as "graded".
    fn how_it_was_ordered(&self) -> String {
        if !self.triage.note.is_empty() {
            return format!("\n  Note: {}.\n", self.triage.note);
        }
        if self.triage.graded > 0 {
            return format!(
                "\n  Severities were re-graded across the whole list, with every defect\n  \
                 in view ({} of {} moved from what the model that found it said).\n",
                self.triage.changed, self.triage.graded
            );
        }
        String::new()
    }

    fn scope_text(&self) -> String {
        match super::common_scope(&self.swept) {
            Ok(scope) => format!("  scope: {}\n", scope.unwrap_or("whole repository")),
            Err(()) => "  WARNING: completed sweeps report different review scopes, so their coverage cannot be interpreted as one run.\n".to_string(),
        }
    }

    fn revision_warnings(&self) -> String {
        let mut commits: Vec<&str> = self
            .swept
            .iter()
            .filter_map(|sweep| sweep.commit.as_deref())
            .filter(|commit| !commit.trim().is_empty())
            .collect();
        commits.sort_unstable();
        commits.dedup();

        let mut out = String::new();
        if commits.len() > 1 {
            let short: Vec<String> = commits
                .iter()
                .map(|commit| commit.chars().take(7).collect())
                .collect();
            out.push_str(&format!(
                "\n  WARNING: these sweeps reviewed {} different revisions ({}), so a finding may cite code that exists only in its own sweep's tree.\n",
                commits.len(),
                short.join(", ")
            ));
        }
        if self
            .swept
            .iter()
            .any(|sweep| sweep.cache_revision.is_none())
        {
            out.push_str(
                "\n  WARNING: at least one completed sweep was unpinned, so revision consistency across this run is unconfirmed.\n",
            );
        }
        out
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from(
            "BUGSLEUTH RUN REPORT\n====================\n\nREVIEW COVERAGE\n---------------\n",
        );
        out.push_str(&self.scope_text());
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
            let salvaged = crate::caveats::salvaged(sweep.salvaged);
            let revision =
                crate::caveats::revision(sweep.commit.as_deref(), sweep.cache_revision.as_deref());
            let usage = sweep
                .usage
                .as_deref()
                .filter(|usage| !usage.trim().is_empty())
                .map(|usage| format!(", usage: {usage}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "  swept: {} lane by {} ({} findings{rejected}; {revision}{usage}){salvaged}\n",
                sweep.lane.title(),
                sweep.model,
                sweep.findings,
            ));
        }
        out.push_str(&self.revision_warnings());
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
        let mut severity = [0; 4];
        for entry in &self.ranked {
            severity[match entry.cluster.severity() {
                Severity::Critical => 0,
                Severity::High => 1,
                Severity::Medium => 2,
                Severity::Low => 3,
            }] += 1;
        }
        out.push_str(&format!(
            "\nSUMMARY\n-------\n  Sweeps completed: {}\n  Findings reported: {total}\n  \
             Distinct defects: {}\n  Severity — Critical: {} | High: {} | Medium: {} | Low: {}\n",
            self.swept.len(),
            self.ranked.len(),
            severity[0],
            severity[1],
            severity[2],
            severity[3],
        ));

        let mut interpretation = self.how_it_was_ordered();

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
            interpretation.push_str(
                "\n  Note: this list spans more than one lane. Severities are relative to\n  \
                 each lane's own mandate and are NOT directly comparable between lanes -\n  \
                 read the ordering within a lane, not across them.\n",
            );
        }
        if !interpretation.is_empty() {
            out.push_str("\nINTERPRETATION\n--------------");
            out.push_str(&interpretation);
        }

        // Both caveats come from one place. Each was once added to a single
        // renderer and forgotten in the other, so the same report read more
        // reassuringly depending only on which command produced it.
        out.push_str("\nLIMITS\n------");
        out.push_str(&crate::caveats::unsandboxed(
            self.swept.iter().map(|sweep| &sweep.model),
            "  ",
        ));
        out.push_str(&crate::caveats::limits("  "));

        out.push_str(&self.findings_text());
        out
    }

    /// Actionable findings followed by observations the code already explains.
    fn findings_text(&self) -> String {
        let known: Vec<_> = self
            .ranked
            .iter()
            .filter(|entry| entry.cluster.acknowledged.is_some())
            .collect();
        let actionable: Vec<_> = self
            .ranked
            .iter()
            .filter(|entry| entry.cluster.acknowledged.is_none())
            .collect();
        let mut out = String::from("\nACTIONABLE FINDINGS\n-------------------\n");
        if !known.is_empty() {
            out.push_str(&format!(
                "\n  {} additional observations are documented in the code as deliberate\n  \
                 decisions. They are listed at the end with what the code says, so you\n  \
                 can disagree with the decision if you want to.\n",
                known.len()
            ));
        }
        if actionable.is_empty() {
            out.push_str("  No actionable defects survived verification.\n");
        }

        for entry in actionable {
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

        out.push_str(&already_documented(&known));
        out
    }
}

/// The findings the code itself already answers, printed with its own words.
///
/// Kept out of the ranked list. Each quote was checked to be really in the file
/// named, so this section can be audited rather than trusted — a reader who
/// disagrees with the decision can go and read it.
fn already_documented(known: &[&bugsleuth_judge::Ranked]) -> String {
    if known.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\nALREADY DOCUMENTED\n------------------\n  Accurate observations the code already \
         answers; each quote below was checked to be really in the file named.\n",
    );
    for entry in known {
        let finding = entry.cluster.representative();
        out.push_str(&format!(
            "\n  - [{}] {}\n    {}:{}\n    the code says: \"{}\"\n",
            entry.cluster.severity().as_str().to_uppercase(),
            finding.title,
            finding.anchor.file,
            finding.anchor.line,
            entry.cluster.acknowledged.as_deref().unwrap_or_default(),
        ));
    }
    out
}

#[cfg(test)]
#[path = "render/tests.rs"]
mod tests;
