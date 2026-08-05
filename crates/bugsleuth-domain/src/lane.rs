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

mod mandates;
use mandates::{CONTRACT_MANDATE, CORRECTNESS_MANDATE, GATE_MANDATE, SECURITY_MANDATE, UX_MANDATE};

/// The lanes. Deliberately a closed enum: adding a lane is a design decision,
/// not a config value, because each one needs a hand-written mandate.
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
    /// The tests and the gate: checks that cannot fail, and green runs that
    /// mean nothing. The only lane whose subject is the evidence itself.
    Gate,
}

impl Lane {
    pub const ALL: [Lane; 5] = [
        Lane::Correctness,
        Lane::Security,
        Lane::Contract,
        Lane::Ux,
        Lane::Gate,
    ];

    pub fn id(self) -> LaneId {
        LaneId::new(self.slug())
    }

    pub fn slug(self) -> &'static str {
        match self {
            Lane::Correctness => "correctness",
            Lane::Security => "security",
            Lane::Contract => "contract",
            Lane::Ux => "ux",
            Lane::Gate => "gate",
        }
    }

    /// Human-readable name for reports.
    pub fn title(self) -> &'static str {
        match self {
            Lane::Correctness => "Correctness",
            Lane::Security => "Security",
            Lane::Contract => "Contract",
            Lane::Ux => "UX",
            Lane::Gate => "Gate",
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
            Lane::Gate => GATE_MANDATE,
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
            "gate" => Ok(Lane::Gate),
            other => Err(UnknownLane(other.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown lane `{0}` (expected one of: correctness, security, contract, ux, gate)")]
pub struct UnknownLane(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lane_parses_back_from_its_own_slug() {
        for lane in Lane::ALL {
            // `.parse().unwrap_or(Lane::Ux)` — which is what this was — cannot
            // fail for the Ux iteration: the fallback is the very value the
            // assertion compares against, so breaking `"ux" => Ok(Lane::Ux)`
            // left this passing while `--lane ux`, a saved settings file and
            // the lane picker all stopped working. Found by BugSleuth's own
            // Gate lane, on its first run against this repository.
            let parsed: Result<Lane, _> = lane.slug().parse();
            assert_eq!(
                parsed.ok(),
                Some(lane),
                "{} does not parse back into its own lane",
                lane.slug()
            );
        }
    }
}
