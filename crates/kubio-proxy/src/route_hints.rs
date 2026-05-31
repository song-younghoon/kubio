use kubio_core::{RouteHintConfig, RouteId};
use std::collections::HashMap;

pub(crate) const DEFAULT_VARY_HEADERS: &[&str] = &["accept", "accept-encoding", "accept-language"];

#[derive(Debug)]
pub(crate) struct RouteHintLookup {
    by_route: HashMap<RouteId, PreparedRouteHint>,
    default_vary_names: Vec<String>,
}

impl RouteHintLookup {
    pub(crate) fn new(hints: &[RouteHintConfig]) -> Self {
        let mut by_route = HashMap::with_capacity(hints.len());
        for hint in hints {
            let route_id = RouteId::new(
                hint.route_match.method.to_ascii_uppercase(),
                hint.route_match.path.clone(),
            );
            by_route
                .entry(route_id)
                .or_insert_with(|| PreparedRouteHint {
                    hint: hint.clone(),
                    vary_names: prepared_vary_names(hint),
                });
        }
        Self {
            by_route,
            default_vary_names: DEFAULT_VARY_HEADERS
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        }
    }

    pub(crate) fn get(&self, route_id: &RouteId) -> Option<&PreparedRouteHint> {
        self.by_route.get(route_id)
    }

    pub(crate) fn default_vary_names(&self) -> &[String] {
        &self.default_vary_names
    }
}

#[derive(Debug)]
pub(crate) struct PreparedRouteHint {
    pub(crate) hint: RouteHintConfig,
    pub(crate) vary_names: Vec<String>,
}

fn prepared_vary_names(hint: &RouteHintConfig) -> Vec<String> {
    if hint.vary.allow.is_empty() {
        DEFAULT_VARY_HEADERS
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    } else {
        hint.vary
            .allow
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_hint_lookup_matches_case_insensitively_and_keeps_first_hint() {
        let first = route_hint("get", "/api/products", Some("first"), &["accept-language"]);
        let duplicate = route_hint("GET", "/api/products", Some("second"), &["x-variant"]);
        let lookup = RouteHintLookup::new(&[first, duplicate]);

        let prepared = lookup
            .get(&RouteId::new("GET", "/api/products"))
            .expect("route hint should be indexed");

        assert_eq!(prepared.hint.display_name(), "first");
        assert_eq!(prepared.vary_names, vec!["accept-language"]);
        assert!(lookup.get(&RouteId::new("POST", "/api/products")).is_none());
    }

    fn route_hint(method: &str, path: &str, name: Option<&str>, vary: &[&str]) -> RouteHintConfig {
        RouteHintConfig {
            name: name.map(ToOwned::to_owned),
            route_match: kubio_core::RouteMatchConfig {
                method: method.to_string(),
                path: path.to_string(),
            },
            freshness: Default::default(),
            query: Default::default(),
            vary: kubio_core::RouteVaryConfig {
                allow: vary.iter().map(|name| (*name).to_string()).collect(),
            },
            stale_if_error: Default::default(),
            safety: Default::default(),
            response_headers: Default::default(),
        }
    }
}
