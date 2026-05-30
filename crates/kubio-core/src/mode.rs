use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Watch,
    Shadow,
    Auto,
}

impl Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Watch => f.write_str("watch"),
            Self::Shadow => f.write_str("shadow"),
            Self::Auto => f.write_str("auto"),
        }
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "watch" => Ok(Self::Watch),
            "shadow" => Ok(Self::Shadow),
            "auto" => Ok(Self::Auto),
            other => Err(format!("unsupported mode `{other}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FreshnessProfile {
    Strict,
    #[default]
    Balanced,
    Relaxed,
}

impl Display for FreshnessProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Strict => f.write_str("strict"),
            Self::Balanced => f.write_str("balanced"),
            Self::Relaxed => f.write_str("relaxed"),
        }
    }
}

impl FromStr for FreshnessProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "strict" => Ok(Self::Strict),
            "balanced" => Ok(Self::Balanced),
            "relaxed" => Ok(Self::Relaxed),
            other => Err(format!("unsupported freshness profile `{other}`")),
        }
    }
}

impl FreshnessProfile {
    pub fn ttl(self) -> Duration {
        match self {
            Self::Strict => Duration::from_secs(5),
            Self::Balanced => Duration::from_secs(30),
            Self::Relaxed => Duration::from_secs(120),
        }
    }
}
