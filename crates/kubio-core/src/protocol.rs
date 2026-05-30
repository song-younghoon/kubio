use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpProtocol {
    #[default]
    Http1,
    Http2,
    Http3,
}

impl Display for HttpProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http1 => f.write_str("http1"),
            Self::Http2 => f.write_str("http2"),
            Self::Http3 => f.write_str("http3"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginProtocolPreference {
    #[default]
    Auto,
    Http1,
    Http2,
    Http3,
}

impl Display for OriginProtocolPreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Http1 => f.write_str("http1"),
            Self::Http2 => f.write_str("http2"),
            Self::Http3 => f.write_str("http3"),
        }
    }
}

impl FromStr for OriginProtocolPreference {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "http1" | "h1" | "http/1.1" => Ok(Self::Http1),
            "http2" | "h2" | "http/2" => Ok(Self::Http2),
            "http3" | "h3" | "http/3" => Ok(Self::Http3),
            other => Err(format!("unsupported origin protocol preference `{other}`")),
        }
    }
}
