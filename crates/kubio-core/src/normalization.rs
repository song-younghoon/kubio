use crate::{short_hash, RouteQueryConfig};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use url::form_urlencoded;

pub fn normalize_path_template(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }

    let mut segments = Vec::new();
    for segment in path.trim_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        let decoded = percent_decode_str(segment)
            .decode_utf8()
            .map(|value| value.to_string())
            .unwrap_or_else(|_| segment.to_string());
        if is_id_like_segment(&decoded) {
            segments.push("{id}".to_string());
        } else {
            segments.push(segment.to_string());
        }
    }

    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

pub fn normalize_query(query: &str) -> String {
    normalize_query_with_config(query, None)
}

pub fn normalize_query_with_config(query: &str, query_config: Option<&RouteQueryConfig>) -> String {
    if query.is_empty() {
        return String::new();
    }

    let mut pairs = form_urlencoded::parse(query.as_bytes())
        .enumerate()
        .map(|(index, (name, value))| (index, name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();

    if let Some(config) = query_config {
        pairs.retain(|(_, name, _)| query_param_allowed(name, config));
    }

    pairs.sort_by(|left, right| match left.1.cmp(&right.1) {
        std::cmp::Ordering::Equal => left.0.cmp(&right.0),
        ordering => ordering,
    });

    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (_, name, value) in pairs {
        serializer.append_pair(&name, &value);
    }
    serializer.finish()
}

fn query_param_allowed(name: &str, config: &RouteQueryConfig) -> bool {
    if config
        .ignore
        .iter()
        .any(|pattern| query_pattern_matches(pattern, name))
    {
        return false;
    }
    if !config.include.is_empty() {
        return config
            .include
            .iter()
            .any(|pattern| query_pattern_matches(pattern, name));
    }
    true
}

pub fn query_pattern_matches(pattern: &str, name: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        pattern == name
    }
}

pub fn is_sensitive_query_param(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "token"
            | "access_token"
            | "auth"
            | "authorization"
            | "session"
            | "password"
            | "secret"
            | "key"
            | "signature"
            | "sig"
    )
}

pub fn is_id_like_segment(segment: &str) -> bool {
    if segment.chars().all(|ch| ch.is_ascii_digit()) {
        return !segment.is_empty();
    }
    if is_uuid_like(segment) {
        return true;
    }
    if is_ulid_like(segment) {
        return true;
    }
    segment.len() >= 16 && segment.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_uuid_like(segment: &str) -> bool {
    let parts = segment.split('-').collect::<Vec<_>>();
    let lengths = [8, 4, 4, 4, 12];
    parts.len() == lengths.len()
        && parts
            .iter()
            .zip(lengths)
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

fn is_ulid_like(segment: &str) -> bool {
    segment.len() == 26
        && segment.chars().all(|ch| {
            matches!(
                ch,
                '0'..='9' | 'A'..='H' | 'J'..='K' | 'M'..='N' | 'P'..='T' | 'V'..='Z'
            )
        })
}

pub fn sensitive_path_score(path: &str) -> u8 {
    let keywords = [
        "me", "user", "users", "account", "profile", "session", "login", "logout", "billing",
        "payment", "checkout", "admin", "token", "oauth",
    ];

    path.trim_matches('/')
        .split('/')
        .filter_map(|segment| {
            percent_decode_str(segment)
                .decode_utf8()
                .ok()
                .map(|decoded| decoded.to_ascii_lowercase())
        })
        .filter(|segment| keywords.iter().any(|keyword| segment == keyword))
        .count()
        .min(u8::MAX as usize) as u8
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathObservation {
    pub segment_count: u16,
    pub id_like_segment_count: u16,
    pub sensitive_path_score: u8,
    pub dynamic_segment_hashes: Vec<String>,
}

pub fn observe_path(path: &str) -> PathObservation {
    let mut observation = PathObservation {
        sensitive_path_score: sensitive_path_score(path),
        ..PathObservation::default()
    };

    for segment in path.trim_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        observation.segment_count = observation.segment_count.saturating_add(1);
        let decoded = percent_decode_str(segment)
            .decode_utf8()
            .map(|value| value.to_string())
            .unwrap_or_else(|_| segment.to_string());
        if is_id_like_segment(&decoded) {
            observation.id_like_segment_count = observation.id_like_segment_count.saturating_add(1);
            observation
                .dynamic_segment_hashes
                .push(short_hash(&decoded));
        }
    }

    observation
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn route_clustering_replaces_ids() {
        assert_eq!(
            normalize_path_template("/api/products/123"),
            "/api/products/{id}"
        );
        assert_eq!(
            normalize_path_template("/api/users/018f4df0-3e42-7046-9d81-a061d74a4c18"),
            "/api/users/{id}"
        );
        assert_eq!(
            normalize_path_template("/api/search?q=phone"),
            "/api/search"
        );
    }

    #[test]
    fn path_observation_hashes_dynamic_values_without_raw_segments() {
        let observation = observe_path("/notice/123");

        assert_eq!(observation.segment_count, 2);
        assert_eq!(observation.id_like_segment_count, 1);
        assert_eq!(observation.sensitive_path_score, 0);
        assert_eq!(observation.dynamic_segment_hashes.len(), 1);
        assert!(!observation.dynamic_segment_hashes[0].contains("123"));
    }

    #[test]
    fn query_normalization_sorts_names_and_preserves_repeats() {
        assert_eq!(normalize_query("b=2&a=1"), "a=1&b=2");
        assert_eq!(normalize_query("b=1&a=0&b=2"), "a=0&b=1&b=2");
    }

    #[test]
    fn query_normalization_applies_route_query_config() {
        let config = RouteQueryConfig {
            include: Vec::new(),
            ignore: vec!["utm_*".to_string(), "gclid".to_string()],
        };

        assert_eq!(
            normalize_query_with_config("b=2&utm_source=x&a=1&gclid=y", Some(&config)),
            "a=1&b=2"
        );
    }

    proptest! {
        #[test]
        fn route_clustering_never_panics(path in "\\PC*") {
            let _ = normalize_path_template(&path);
        }

        #[test]
        fn query_normalization_is_stable_for_parameter_order(a in "[A-Za-z0-9]{0,16}", b in "[A-Za-z0-9]{0,16}") {
            let left = format!("b={b}&a={a}");
            let right = format!("a={a}&b={b}");

            prop_assert_eq!(normalize_query(&left), normalize_query(&right));
        }
    }
}
