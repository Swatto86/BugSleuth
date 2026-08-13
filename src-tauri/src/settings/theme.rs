//! The three theme values the desktop control and CSS can represent.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            "system" => Self::System,
            _ => Self::System,
        })
    }
}
