use http::{header, HeaderMap};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheControlClass {
    Absent,
    Public,
    Private,
    NoStore,
    NoCache,
    Other,
}

impl CacheControlClass {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let Some(value) = headers.get(header::CACHE_CONTROL) else {
            return Self::Absent;
        };
        let Ok(value) = value.to_str() else {
            return Self::Other;
        };

        let directives = value
            .split(',')
            .map(|part| {
                part.trim()
                    .split_once('=')
                    .map(|(name, _)| name)
                    .unwrap_or_else(|| part.trim())
                    .to_ascii_lowercase()
            })
            .collect::<Vec<_>>();

        if directives.iter().any(|directive| directive == "no-store") {
            Self::NoStore
        } else if directives.iter().any(|directive| directive == "private") {
            Self::Private
        } else if directives.iter().any(|directive| directive == "no-cache") {
            Self::NoCache
        } else if directives.iter().any(|directive| directive == "public") {
            Self::Public
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaryClass {
    Absent,
    Supported(Vec<String>),
    Wildcard,
    Unsupported(Vec<String>),
}

impl VaryClass {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let Some(value) = headers.get(header::VARY) else {
            return Self::Absent;
        };
        let Ok(value) = value.to_str() else {
            return Self::Unsupported(vec!["<invalid>".to_string()]);
        };
        let names = value
            .split(',')
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if names.is_empty() {
            return Self::Absent;
        }
        if names.iter().any(|name| name == "*") {
            return Self::Wildcard;
        }
        let unsupported = names
            .iter()
            .filter(|name| {
                !matches!(
                    name.as_str(),
                    "accept" | "accept-encoding" | "accept-language"
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        if unsupported.is_empty() {
            Self::Supported(names)
        } else {
            Self::Unsupported(unsupported)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentTypeClass {
    Json,
    Text,
    Binary,
    Unknown,
}

impl ContentTypeClass {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let Some(value) = headers.get(header::CONTENT_TYPE) else {
            return Self::Unknown;
        };
        let Ok(value) = value.to_str() else {
            return Self::Unknown;
        };
        let value = value.to_ascii_lowercase();
        if value.contains("json") {
            Self::Json
        } else if value.starts_with("text/") {
            Self::Text
        } else {
            Self::Binary
        }
    }
}
