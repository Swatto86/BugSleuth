//! Newtypes for the domain's primitive identifiers.
//!
//! These exist so a `ModelId` can never be passed where a `LaneId` is expected —
//! all four would otherwise be bare `String`s and the compiler would not care.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }
    };
}

string_id!(
    ModelId,
    "Identifies a configured model, e.g. `claude:sonnet` or `codex:gpt-5.6`."
);
string_id!(LaneId, "Identifies a review lane, e.g. `correctness`.");
string_id!(
    FindingId,
    "Identifies a single finding within a run. Stable once assigned."
);
string_id!(RunId, "Identifies one sweep of a repository.");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_json_as_bare_strings() {
        let model = ModelId::new("claude:sonnet");
        let json = serde_json::to_string(&model).unwrap_or_default();
        assert_eq!(json, "\"claude:sonnet\"");
        let back: ModelId = serde_json::from_str(&json).unwrap_or_else(|_| ModelId::new(""));
        assert_eq!(back, model);
    }
}
