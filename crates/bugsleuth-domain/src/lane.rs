//! Review lanes.
//!
//! A lane is a review *mandate*: a distinct brief plus the slice of the repo it
//! applies to. Lanes exist because one generic "find bugs" prompt collapses
//! toward the same handful of findings no matter which model you ask. Giving
//! each sweep a narrow, different mandate manufactures the diversity the whole
//! tool depends on.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ids::LaneId;

/// The four v1 lanes. Deliberately a closed enum: adding a lane is a design
/// decision, not a config value, because each one needs a hand-written mandate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    /// Logic errors, edge cases, error handling, concurrency.
    Correctness,
    /// IPC surface, injection, secrets, authorization, dependency risk.
    Security,
    /// Command signatures, frontend/backend type drift, wire formats.
    Contract,
    /// Behavioural UI defects only — never aesthetics.
    Ux,
}

impl Lane {
    pub const ALL: [Lane; 4] = [Lane::Correctness, Lane::Security, Lane::Contract, Lane::Ux];

    pub fn id(self) -> LaneId {
        LaneId::new(self.slug())
    }

    pub fn slug(self) -> &'static str {
        match self {
            Lane::Correctness => "correctness",
            Lane::Security => "security",
            Lane::Contract => "contract",
            Lane::Ux => "ux",
        }
    }

    /// Human-readable name for reports.
    pub fn title(self) -> &'static str {
        match self {
            Lane::Correctness => "Correctness",
            Lane::Security => "Security",
            Lane::Contract => "Contract",
            Lane::Ux => "UX",
        }
    }

    /// The lane's review mandate, injected verbatim into the model's brief.
    ///
    /// Each mandate states what is in scope *and* what is explicitly out of
    /// scope. The out-of-scope half matters as much as the in-scope half: the
    /// failure mode these lanes exist to prevent is every lane drifting back to
    /// the same generic findings.
    pub fn mandate(self) -> &'static str {
        match self {
            Lane::Correctness => CORRECTNESS_MANDATE,
            Lane::Security => SECURITY_MANDATE,
            Lane::Contract => CONTRACT_MANDATE,
            Lane::Ux => UX_MANDATE,
        }
    }
}

impl fmt::Display for Lane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

impl std::str::FromStr for Lane {
    type Err = UnknownLane;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "correctness" => Ok(Lane::Correctness),
            "security" => Ok(Lane::Security),
            "contract" => Ok(Lane::Contract),
            "ux" => Ok(Lane::Ux),
            other => Err(UnknownLane(other.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown lane `{0}` (expected one of: correctness, security, contract, ux)")]
pub struct UnknownLane(pub String);

const CORRECTNESS_MANDATE: &str = "\
Find defects where the code does something other than what it plainly intends: \
logic errors, off-by-one and boundary mistakes, unhandled edge cases (empty, \
zero, negative, overflow, unicode), swallowed or misreported errors, panics on \
reachable input, resource leaks, and concurrency faults (races, deadlocks, \
lost wakeups, non-atomic read-modify-write).

# Two kinds you will miss unless you look for them deliberately

Everything above is visible in code that is *present*. These two are not, and \
they are the ones that get past reviewers.

**Destroy-before-commit.** Look at every sequence that replaces stored state — \
a file, a credential, a database row, a cache. Does it delete or clear the old \
value *before* the new one is safely written? If the process died between those \
two steps, or the second step returned an error, what is left? Code like \
`clear()?; write(new)?` reads perfectly and destroys the only copy of something \
whenever the write fails. This is not a concurrency bug and you will not find \
it by looking for races; it is an ordering bug, visible only when you ask what \
an interruption between two lines would leave behind.

**Received but never handled.** Look for values the code takes in and then \
silently drops: a field parsed from a response and never stored, an enum variant \
or status the caller never branches on, an optional flag that only one of \
several paths applies. The defect is the *absence* of a branch, so there is no \
wrong line to spot — you have to compare what comes in against what is acted \
on, and notice the gap. Ask of each incoming field: where does this end up, and \
what happens when only this one changes?

Both of these are found by asking what is missing, not by reading for what is \
wrong. Spend real effort on them.

OUT OF SCOPE for this lane: security vulnerabilities, API/type-contract \
mismatches, and anything about the user interface. Other reviewers cover those. \
Do not report style, naming, formatting, or 'this could be cleaner'.";

const SECURITY_MANDATE: &str = "\
Find defects an attacker could use: command/SQL/path injection, missing or \
incorrect authorization checks, secrets or credentials in source, logs, or \
error messages, unsafe deserialization, TOCTOU on file paths, over-broad \
permissions and capability grants, unvalidated input crossing a trust boundary \
(IPC, subprocess arguments, HTTP handlers, file paths), and dependency risk.

# Do not stop at the first one

The most likely way you will miss a real hole is by finding the loudest \
instance of a sink and moving on. Measured against known defects, two \
reviewers each found one unescaped `innerHTML` assignment, reported it, and \
never looked at the same file's other one — which was also unescaped and also \
reachable from remote data.

So when you find any sink — an HTML injection point, a path join, a query \
built by concatenation, a subprocess argument, a deserialization — **enumerate \
every other use of that same sink in the codebase and check each one \
separately**. Some will be escaped and some will not, and the escaped ones are \
what make the unescaped one easy to miss. A helper that *builds* a string \
which is later assigned to a sink is part of that sink: follow it back.

Report each unsafe use as its own finding. One finding that says \
'and there are others like it' cannot be fixed one commit at a time.

OUT OF SCOPE for this lane: ordinary logic bugs with no attacker angle, \
type-contract drift, and user-interface behaviour. Do not report theoretical \
hardening ideas with no concrete exploitation path.";

const CONTRACT_MANDATE: &str = "\
Find places where two sides of an interface disagree: a function or command \
whose signature does not match its callers, a serialized type whose field \
names, casing, optionality, or numeric width differ between producer and \
consumer, an enum variant one side can emit and the other cannot parse, a \
version-skew hazard in a persisted format, and error shapes that the caller \
cannot actually distinguish.

# The same rule, written down twice

A mismatch is not always a type. Look for a rule expressed on both sides of a \
boundary in different words: a maximum size checked in the frontend and again \
in the backend, a timeout both ends assume, a retry count, a page size, a \
field-length limit, a regular expression duplicated in two languages. Nothing \
makes those two values agree, and nothing fails when they drift — the wrong \
side simply wins, silently.

Measured against a known defect, a frontend refused attachments over one limit \
while the backend enforced a smaller one, so files that passed the check the \
user could see failed at send with a raw error. Both numbers were plain \
constants, in the repository, ten minutes apart.

Read every literal limit you find, and go looking for its counterpart on the \
other side. When you find one, report both values and which one wins.

OUT OF SCOPE for this lane: internal logic errors, security issues, and \
interface aesthetics. A finding here must name BOTH sides of the mismatch.";

const UX_MANDATE: &str = "\
Find BEHAVIOURAL user-interface defects only: a destructive action with no \
confirmation, an operation with no loading or progress state, a failure that \
is swallowed so the user sees nothing, a state the user can reach and cannot \
get out of, an action reachable only by mouse with no keyboard path, and \
focus/announcement gaps that make a flow unusable with a screen reader.

Before claiming an action is mouse-only, check WHAT ELEMENT the handler is \
attached to. A native focusable control — a real <button>, a link, anything \
with tabindex — gets keyboard paths from the platform itself: Enter and Space \
fire click, and Shift+F10 or the Menu key fires contextmenu, reaching the very \
listener you are looking at. 'No keyboard path' is only true when the target \
is a non-focusable element (a plain div with no tabindex) or the handler \
filters to a pointer device. One graded report got this exactly half right: \
context-menu actions on plain <div> rows really were unreachable, and the \
identical claim about native <button> folder rows was false.

HARD RULE: every finding must name the specific code that has to change. \
Aesthetic opinions — spacing, colour, wording, 'this should be bigger', \
'this could be more modern' — are OUT OF SCOPE and actively harmful to this \
report. If you cannot point at code, do not report it.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lane_parses_back_from_its_own_slug() {
        for lane in Lane::ALL {
            let parsed: Lane = lane.slug().parse().unwrap_or(Lane::Ux);
            assert_eq!(parsed, lane);
        }
    }

    #[test]
    fn the_correctness_mandate_asks_for_the_two_classes_it_measurably_missed() {
        // Graded against known defects, the correctness lane found 1 of 3 with
        // three vendors reading the exact files. Both misses were absence
        // defects: a delete-then-write ordering, and a field received and never
        // handled. Every class the mandate had listed was visible in code that
        // was present, so neither miss was ever being looked for.
        let mandate = Lane::Correctness.mandate();
        assert!(
            mandate.contains("before"),
            "nothing asks about destroying state before the replacement is committed"
        );
        assert!(
            mandate.contains("never handled") || mandate.contains("silently drops"),
            "nothing asks about a value received and then dropped"
        );
        assert!(
            mandate.contains("what is missing"),
            "nothing tells the reviewer to read for absence rather than for error"
        );
    }

    #[test]
    fn the_ux_mandate_warns_about_native_keyboard_paths_it_measurably_missed() {
        // Independently graded on a real mail client, the one false positive in
        // thirteen findings claimed folder actions were mouse-only — but the
        // rows were native <button> elements, which the platform already gives
        // Enter/Space activation and a Shift+F10 context-menu path. The sibling
        // finding about plain <div> rows was correct. The difference is what
        // element the handler targets, so the mandate now says to check it.
        let mandate = Lane::Ux.mandate();
        assert!(
            mandate.contains("WHAT ELEMENT"),
            "nothing tells the reviewer to check the handler's target element"
        );
        assert!(
            mandate.contains("Shift+F10"),
            "nothing names the platform's own keyboard path to a context menu"
        );
    }

    #[test]
    fn the_security_mandate_asks_for_the_other_uses_of_a_sink_it_measurably_missed() {
        // Graded against a known injection defect, two vendors each found one
        // unescaped innerHTML assignment, reported it, and never looked at the
        // same file's other one - which was also unescaped and also reachable
        // from remote data. Finding the loudest instance and stopping was the
        // whole miss.
        let mandate = Lane::Security.mandate();
        assert!(
            mandate.contains("enumerate"),
            "nothing tells the reviewer to sweep the other uses of a sink it found"
        );
        assert!(
            mandate.contains("Do not stop at the first"),
            "nothing warns against stopping at the loudest instance"
        );
    }

    #[test]
    fn the_contract_mandate_asks_about_a_rule_written_down_on_both_sides() {
        // The measured miss was two plain constants: a frontend size guard and
        // the smaller backend budget it was meant to pre-screen for. Both were
        // in the repository; neither side made them agree.
        let mandate = Lane::Contract.mandate();
        assert!(
            mandate.contains("written down twice") || mandate.contains("expressed on both sides"),
            "nothing asks about one rule duplicated across a boundary"
        );
        assert!(
            mandate.contains("report both values"),
            "nothing asks for both sides of a duplicated limit"
        );
    }
}
