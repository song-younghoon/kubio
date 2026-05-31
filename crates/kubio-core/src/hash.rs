use crate::{
    is_default_volatile_response_header, is_legacy_volatile_header,
    is_response_header_candidate_eligible, response_header_pattern_matches,
    HeaderEquivalenceSource, ResponseHeaderEquivalenceConfig, RouteResponseHeadersConfig,
    RESPONSE_HEADER_FINGERPRINT_POLICY_VERSION,
};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub fn stable_header_hash(headers: &HeaderMap) -> String {
    stable_header_fingerprint(
        headers,
        &ResponseHeaderEquivalenceConfig::default(),
        None,
        &[],
    )
    .hash
}

pub fn legacy_stable_header_hash(headers: &HeaderMap) -> String {
    let mut stable = headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str().to_ascii_lowercase();
            if is_legacy_volatile_header(&name) {
                return None;
            }
            value
                .to_str()
                .ok()
                .map(|value| (name, value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    stable.sort();

    let mut material = String::new();
    for (name, value) in stable {
        material.push_str(&name);
        material.push(':');
        material.push_str(&value);
        material.push('\n');
    }
    short_hash(&material)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderFingerprintPolicy {
    pub version: u16,
    pub default_volatile_add: Vec<String>,
    pub default_volatile_block: Vec<String>,
    pub force_include: Vec<String>,
    pub verified_ignored: Vec<String>,
}

impl HeaderFingerprintPolicy {
    pub fn from_config(
        config: &ResponseHeaderEquivalenceConfig,
        route_headers: Option<&RouteResponseHeadersConfig>,
        verified_ignored_names: &[String],
    ) -> Self {
        Self {
            version: if config.enabled {
                RESPONSE_HEADER_FINGERPRINT_POLICY_VERSION
            } else {
                1
            },
            default_volatile_add: config.default_volatile.add.clone(),
            default_volatile_block: config.default_volatile.block.clone(),
            force_include: route_headers
                .map(|headers| headers.force_include.clone())
                .unwrap_or_default(),
            verified_ignored: verified_ignored_names.to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderFingerprintResult {
    pub hash: String,
    pub policy_version: u16,
    pub included_names: Vec<String>,
    pub ignored_names: Vec<HeaderIgnoreRecord>,
    pub suppressed_on_hit_names: Vec<String>,
    pub candidate_observations: Vec<ResponseHeaderObservation>,
}

impl HeaderFingerprintResult {
    pub fn ignored_header_names(&self) -> Vec<String> {
        self.ignored_names
            .iter()
            .map(|record| record.name.clone())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderIgnoreRecord {
    pub name: String,
    pub source: HeaderEquivalenceSource,
    pub suppressed_on_hit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseHeaderObservation {
    pub name: String,
    pub value_hash: String,
    pub fingerprint_hash_without_header: String,
    pub default_ignored: bool,
    pub ignored: bool,
    pub suppressed_on_hit: bool,
    pub sensitive: bool,
    pub source: HeaderEquivalenceSource,
}

pub fn stable_header_fingerprint(
    headers: &HeaderMap,
    config: &ResponseHeaderEquivalenceConfig,
    route_headers: Option<&RouteResponseHeadersConfig>,
    verified_ignored_names: &[String],
) -> HeaderFingerprintResult {
    let mut included = Vec::new();
    let mut ignored = Vec::new();
    let mut suppressed_on_hit = Vec::new();
    let mut candidate_observations = Vec::new();
    let mut ignored_name_set = HashSet::new();
    let force_include = force_include_names(config, route_headers);
    let verified_ignored = verified_ignored_names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        let Ok(value) = value.to_str() else {
            continue;
        };
        let value = value.trim().to_string();
        let forced = force_include.contains(&lower);
        let legacy_ignored = !config.enabled && !forced && is_legacy_volatile_header(&lower);
        let default_ignored = config.enabled
            && !forced
            && config.verified_ignore.auto_apply_known_metadata
            && default_volatile_enabled(config, &lower)
            && is_default_volatile_response_header(&lower);
        let route_or_verified_ignored = config.enabled
            && !forced
            && verified_ignored.contains(&lower)
            && config.verified_ignore.enabled;
        let route_allowed_ignored = config.enabled
            && !forced
            && config.verified_ignore.enabled
            && route_headers
                .map(|headers| {
                    headers.verified_ignore.enabled
                        && headers
                            .verified_ignore
                            .allow
                            .iter()
                            .any(|pattern| response_header_pattern_matches(pattern, &lower))
                        && is_response_header_candidate_eligible(&lower)
                })
                .unwrap_or(false);
        let added_default_ignored = config.enabled
            && !forced
            && config
                .default_volatile
                .add
                .iter()
                .any(|pattern| response_header_pattern_matches(pattern, &lower));

        if legacy_ignored
            || default_ignored
            || route_or_verified_ignored
            || route_allowed_ignored
            || added_default_ignored
        {
            let source = if route_allowed_ignored {
                HeaderEquivalenceSource::RouteHint
            } else if route_or_verified_ignored {
                HeaderEquivalenceSource::VerifiedEvidence
            } else if added_default_ignored {
                HeaderEquivalenceSource::GlobalConfig
            } else {
                HeaderEquivalenceSource::DefaultPolicy
            };
            let suppress = should_suppress_on_hit(
                config,
                route_headers,
                &lower,
                legacy_ignored || default_ignored || added_default_ignored,
                route_or_verified_ignored || route_allowed_ignored,
            );
            ignored.push(HeaderIgnoreRecord {
                name: lower.clone(),
                source,
                suppressed_on_hit: suppress,
            });
            ignored_name_set.insert(lower.clone());
            if suppress {
                suppressed_on_hit.push(lower);
            }
            candidate_observations.push(ResponseHeaderObservation {
                name: name.as_str().to_ascii_lowercase(),
                value_hash: short_hash(&value),
                fingerprint_hash_without_header: String::new(),
                default_ignored: legacy_ignored || default_ignored || added_default_ignored,
                ignored: route_or_verified_ignored || route_allowed_ignored,
                suppressed_on_hit: suppress,
                sensitive: false,
                source,
            });
            continue;
        }

        included.push((lower, value));
    }

    included.sort();
    let hash = stable_hash_from_pairs(&included);
    let included_names = included
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();

    if config.enabled && config.verified_ignore.enabled {
        for (name, value) in headers {
            let lower = name.as_str().to_ascii_lowercase();
            if ignored_name_set.contains(&lower) || !is_response_header_candidate_eligible(&lower) {
                continue;
            }
            let Ok(value) = value.to_str() else {
                continue;
            };
            let mut without = Vec::new();
            for (candidate_name, candidate_value) in headers {
                let candidate_lower = candidate_name.as_str().to_ascii_lowercase();
                if candidate_lower == lower || ignored_name_set.contains(&candidate_lower) {
                    continue;
                }
                if let Ok(candidate_value) = candidate_value.to_str() {
                    without.push((candidate_lower, candidate_value.trim().to_string()));
                }
            }
            without.sort();
            candidate_observations.push(ResponseHeaderObservation {
                name: lower,
                value_hash: short_hash(value.trim()),
                fingerprint_hash_without_header: stable_hash_from_pairs(&without),
                default_ignored: false,
                ignored: false,
                suppressed_on_hit: false,
                sensitive: false,
                source: HeaderEquivalenceSource::VerifiedEvidence,
            });
        }
    }

    HeaderFingerprintResult {
        hash,
        policy_version: if config.enabled {
            RESPONSE_HEADER_FINGERPRINT_POLICY_VERSION
        } else {
            1
        },
        included_names,
        ignored_names: ignored,
        suppressed_on_hit_names: suppressed_on_hit,
        candidate_observations,
    }
}

pub fn should_suppress_response_header_on_hit(
    config: &ResponseHeaderEquivalenceConfig,
    route_headers: Option<&RouteResponseHeadersConfig>,
    name: &str,
    stored_suppressed_names: &[String],
) -> bool {
    let lower = name.to_ascii_lowercase();
    if !config.enabled {
        return stored_suppressed_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&lower));
    }
    if lower == "date" && config.serve.preserve_date {
        return false;
    }
    if route_headers
        .map(|headers| {
            headers
                .preserve_on_hit
                .iter()
                .any(|pattern| response_header_pattern_matches(pattern, &lower))
        })
        .unwrap_or(false)
    {
        return false;
    }
    stored_suppressed_names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&lower))
        || (config.serve.strip_volatile_on_hit && auto_applied_default_volatile(config, &lower))
        || (config.serve.strip_volatile_on_hit && added_default_volatile(config, &lower))
}

fn should_suppress_on_hit(
    config: &ResponseHeaderEquivalenceConfig,
    route_headers: Option<&RouteResponseHeadersConfig>,
    name: &str,
    volatile_ignored: bool,
    verified_ignored: bool,
) -> bool {
    if !config.enabled {
        return false;
    }
    if name.eq_ignore_ascii_case("date") && config.serve.preserve_date {
        return false;
    }
    !route_headers
        .map(|headers| {
            headers
                .preserve_on_hit
                .iter()
                .any(|pattern| response_header_pattern_matches(pattern, name))
        })
        .unwrap_or(false)
        && ((volatile_ignored && config.serve.strip_volatile_on_hit)
            || (verified_ignored && config.serve.strip_verified_ignored_on_hit))
}

fn force_include_names(
    config: &ResponseHeaderEquivalenceConfig,
    route_headers: Option<&RouteResponseHeadersConfig>,
) -> HashSet<String> {
    let mut names = config
        .default_volatile
        .block
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if let Some(route_headers) = route_headers {
        names.extend(
            route_headers
                .force_include
                .iter()
                .map(|name| name.to_ascii_lowercase()),
        );
    }
    names
}

fn default_volatile_enabled(config: &ResponseHeaderEquivalenceConfig, name: &str) -> bool {
    is_default_volatile_response_header(name)
        && !config
            .default_volatile
            .block
            .iter()
            .any(|pattern| response_header_pattern_matches(pattern, name))
}

fn auto_applied_default_volatile(config: &ResponseHeaderEquivalenceConfig, name: &str) -> bool {
    config.verified_ignore.auto_apply_known_metadata && default_volatile_enabled(config, name)
}

fn added_default_volatile(config: &ResponseHeaderEquivalenceConfig, name: &str) -> bool {
    config
        .default_volatile
        .add
        .iter()
        .any(|pattern| response_header_pattern_matches(pattern, name))
}

fn stable_hash_from_pairs(pairs: &[(String, String)]) -> String {
    let mut material = String::new();
    for (name, value) in pairs {
        material.push_str(name);
        material.push(':');
        material.push_str(value);
        material.push('\n');
    }
    short_hash(&material)
}

pub fn body_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn short_hash(value: &str) -> String {
    let digest = blake3::hash(value.as_bytes()).to_hex().to_string();
    digest[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volatile_headers_are_excluded_from_hash() {
        let mut first = HeaderMap::new();
        first.insert("date", "today".parse().unwrap());
        first.insert("x-response-id", "first".parse().unwrap());
        first.insert("content-type", "application/json".parse().unwrap());

        let mut second = HeaderMap::new();
        second.insert("date", "tomorrow".parse().unwrap());
        second.insert("x-response-id", "second".parse().unwrap());
        second.insert("content-type", "application/json".parse().unwrap());

        assert_eq!(stable_header_hash(&first), stable_header_hash(&second));
    }

    #[test]
    fn force_include_keeps_default_volatile_header_in_hash() {
        let mut first = HeaderMap::new();
        first.insert("x-response-id", "first".parse().unwrap());
        let mut second = HeaderMap::new();
        second.insert("x-response-id", "second".parse().unwrap());
        let route_headers = RouteResponseHeadersConfig {
            force_include: vec!["x-response-id".to_string()],
            ..Default::default()
        };

        assert_ne!(
            stable_header_fingerprint(
                &first,
                &ResponseHeaderEquivalenceConfig::default(),
                Some(&route_headers),
                &[]
            )
            .hash,
            stable_header_fingerprint(
                &second,
                &ResponseHeaderEquivalenceConfig::default(),
                Some(&route_headers),
                &[]
            )
            .hash
        );
    }

    #[test]
    fn known_metadata_auto_apply_can_be_disabled() {
        let mut config = ResponseHeaderEquivalenceConfig::default();
        config.verified_ignore.auto_apply_known_metadata = false;

        let mut first = HeaderMap::new();
        first.insert("x-response-id", "first".parse().unwrap());
        let mut second = HeaderMap::new();
        second.insert("x-response-id", "second".parse().unwrap());

        let first_fingerprint = stable_header_fingerprint(&first, &config, None, &[]);
        let second_fingerprint = stable_header_fingerprint(&second, &config, None, &[]);

        assert_ne!(first_fingerprint.hash, second_fingerprint.hash);
        assert!(first_fingerprint.ignored_names.is_empty());
        assert!(!should_suppress_response_header_on_hit(
            &config,
            None,
            "x-response-id",
            &[],
        ));
    }

    #[test]
    fn body_changes_alter_fingerprint_hash() {
        assert_ne!(body_hash(b"one"), body_hash(b"two"));
    }
}
