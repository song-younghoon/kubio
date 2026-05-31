use crate::{normalize_path_template, short_hash, RouteHintConfig};
use http::Method;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RouteId {
    pub method: String,
    pub template: String,
}

impl RouteId {
    pub fn new(method: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            template: template.into(),
        }
    }

    pub fn from_method_path(method: &Method, path: &str) -> Self {
        Self::new(method.as_str(), normalize_path_template(path))
    }

    pub fn as_label(&self) -> String {
        format!("{} {}", self.method, self.template)
    }

    pub fn hash(&self) -> String {
        short_hash(&self.as_label())
    }
}

impl Display for RouteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.template)
    }
}

pub fn matching_route_hint<'a>(
    route_id: &RouteId,
    hints: &'a [RouteHintConfig],
) -> Option<&'a RouteHintConfig> {
    hints.iter().find(|hint| hint.matches(route_id))
}
