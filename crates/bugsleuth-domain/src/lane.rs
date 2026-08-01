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

OUT OF SCOPE for this lane: security vulnerabilities, API/type-contract \
mismatches, and anything about the user interface. Other reviewers cover those. \
Do not report style, naming, formatting, or 'this could be cleaner'.";

const SECURITY_MANDATE: &str = "\
Find defects an attacker could use: command/SQL/path injection, missing or \
incorrect authorization checks, secrets or credentials in source, logs, or \
error messages, unsafe deserialization, TOCTOU on file paths, over-broad \
permissions and capability grants, unvalidated input crossing a trust boundary \
(IPC, subprocess arguments, HTTP handlers, file paths), and dependency risk.

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

OUT OF SCOPE for this lane: internal logic errors, security issues, and \
interface aesthetics. A finding here must name BOTH sides of the mismatch.";

const UX_MANDATE: &str = "\
Find BEHAVIOURAL user-interface defects only: a destructive action with no \
confirmation, an operation with no loading or progress state, a failure that \
is swallowed so the user sees nothing, a state the user can reach and cannot \
get out of, an action reachable only by mouse with no keyboard path, and \
focus/announcement gaps that make a flow unusable with a screen reader.

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
}
