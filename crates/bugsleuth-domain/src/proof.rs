//! Proof-carrying findings.
//!
//! A finding backed by a test that fails against the current code is
//! qualitatively different from one that is merely asserted: it converts "a
//! model believes this is a bug" into a fact a machine checked. That matters
//! most for someone who cannot read the code, because to them a well-written
//! hallucination and a real defect are indistinguishable.
//!
//! Nothing here trusts the model's own account of what happened. `ProofClaim` is
//! what it said; `ProofVerdict` is what we observed by running the tests
//! ourselves.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The model's own report after attempting to demonstrate a defect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofClaim {
    /// Whether it believes it wrote a test that fails because of the defect.
    pub wrote_failing_test: bool,
    /// Cargo test filter for the test it added, e.g. `html::tests::spaced_src`.
    #[serde(default)]
    pub test_name: String,
    /// File the test was added to.
    #[serde(default)]
    pub test_file: String,
    /// What it saw when it ran the test.
    #[serde(default)]
    pub observed: String,
    /// If it could not produce a failing test, why not. This is as valuable as a
    /// success: a defect nobody can write a test for is a defect worth doubting.
    #[serde(default)]
    pub obstacle: String,
}

/// What actually happened when *we* ran the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofVerdict {
    /// The new test fails, and every pre-existing test still passes. The defect
    /// is demonstrated and the model did not reach the result by sabotage.
    Proved,
    /// The new test passes, so it demonstrates nothing about the defect.
    TestDoesNotFail,
    /// The new test fails, but so do pre-existing tests: the model changed
    /// production code rather than only adding a test, so the failure is not
    /// evidence about the original defect.
    SuiteSabotaged,
    /// The model reported a test that does not exist or cannot be selected.
    TestNotFound,
    /// The tree does not compile after the model's changes.
    DidNotBuild,
    /// The model did not produce a test at all.
    NoTestWritten,
    /// The test run was killed for running too long.
    TimedOut,
}

impl ProofVerdict {
    /// Whether this finding may be presented as proven.
    pub fn is_proof(self) -> bool {
        self == ProofVerdict::Proved
    }

    pub fn describe(self) -> &'static str {
        match self {
            ProofVerdict::Proved => "proved by a failing test",
            ProofVerdict::TestDoesNotFail => "the test written for it passes, so it proves nothing",
            ProofVerdict::SuiteSabotaged => {
                "the test fails, but the model also broke existing tests, so the failure is not evidence"
            }
            ProofVerdict::TestNotFound => "the reported test could not be found",
            ProofVerdict::DidNotBuild => "the code does not compile after the attempt",
            ProofVerdict::NoTestWritten => "no test was produced",
            ProofVerdict::TimedOut => "the test run did not finish",
        }
    }
}

/// JSON Schema for [`ProofClaim`].
pub fn proof_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["wrote_failing_test", "test_name", "test_file", "observed", "obstacle"],
        "properties": {
            "wrote_failing_test": {
                "type": "boolean",
                "description": "True only if you added a test that FAILS because of this defect and you ran it and saw it fail."
            },
            "test_name": {
                "type": "string",
                "description": "The cargo test filter that selects only your new test, e.g. html::tests::my_new_test. Letters, digits, underscores and :: only. Empty if you wrote no test."
            },
            "test_file": {
                "type": "string",
                "description": "Repository-relative path of the file you added the test to. Empty if you wrote no test."
            },
            "observed": {
                "type": "string",
                "description": "What you actually saw when you ran the test — quote the assertion failure."
            },
            "obstacle": {
                "type": "string",
                "description": "If you could NOT write a failing test, explain precisely why. Empty otherwise. An honest obstacle is more useful than a test that does not really demonstrate the defect."
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clean_proof_counts_as_proof() {
        assert!(ProofVerdict::Proved.is_proof());
        for verdict in [
            ProofVerdict::TestDoesNotFail,
            ProofVerdict::SuiteSabotaged,
            ProofVerdict::TestNotFound,
            ProofVerdict::DidNotBuild,
            ProofVerdict::NoTestWritten,
            ProofVerdict::TimedOut,
        ] {
            assert!(!verdict.is_proof(), "{verdict:?} must not count as proof");
        }
    }

    #[test]
    fn the_schema_required_set_builds_a_claim() {
        let schema = proof_schema();
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        let mut object = serde_json::Map::new();
        for name in required.iter().filter_map(Value::as_str) {
            let value = if name == "wrote_failing_test" {
                json!(true)
            } else {
                json!("x")
            };
            object.insert(name.to_string(), value);
        }
        assert!(serde_json::from_value::<ProofClaim>(Value::Object(object)).is_ok());
    }
}
