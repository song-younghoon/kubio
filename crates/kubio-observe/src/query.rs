use kubio_core::{QueryEquivalenceClass, QueryEquivalenceConfig, ResponseFingerprint};
use std::collections::{HashMap, HashSet};

use crate::records::QueryParamRecord;
use crate::snapshot::QueryParamSnapshot;

pub(crate) const QUERY_VALUE_SAMPLE_LIMIT: usize = 32;
const QUERY_SUGGESTION_MIN_FINGERPRINTS: u64 = 2;

#[derive(Debug, Clone)]
pub(crate) struct QueryParamStats {
    pub(crate) name: String,
    pub(crate) seen_count: u64,
    pub(crate) configured_action: String,
    pub(crate) fingerprint_sensitive: bool,
    pub(crate) sensitive: bool,
    pub(crate) value_hashes: HashSet<String>,
    pub(crate) value_hash_overflow: bool,
    pub(crate) fingerprints_by_value: HashMap<String, String>,
    pub(crate) fingerprint_hashes: HashSet<String>,
    pub(crate) fingerprint_observations: u64,
    pub(crate) fingerprint_mismatches: u64,
    pub(crate) suggestion_event_emitted: bool,
}

impl QueryParamStats {
    pub(crate) fn new(name: String) -> Self {
        Self {
            name,
            seen_count: 0,
            configured_action: "observe".to_string(),
            fingerprint_sensitive: false,
            sensitive: false,
            value_hashes: HashSet::new(),
            value_hash_overflow: false,
            fingerprints_by_value: HashMap::new(),
            fingerprint_hashes: HashSet::new(),
            fingerprint_observations: 0,
            fingerprint_mismatches: 0,
            suggestion_event_emitted: false,
        }
    }

    pub(crate) fn record_seen(&mut self, param: &QueryParamRecord) {
        self.seen_count += 1;
        self.configured_action.clone_from(&param.configured_action);
        self.sensitive |= param.sensitive;
        if self.sensitive {
            return;
        }
        if let Some(value_hash) = param.value_hash.as_ref() {
            if self.value_hashes.contains(value_hash) {
                return;
            }
            if self.value_hashes.len() < QUERY_VALUE_SAMPLE_LIMIT {
                self.value_hashes.insert(value_hash.clone());
            } else {
                self.value_hash_overflow = true;
            }
        }
    }

    pub(crate) fn record_fingerprint(&mut self, param: &QueryParamRecord, fingerprint_hash: &str) {
        self.configured_action.clone_from(&param.configured_action);
        self.sensitive |= param.sensitive;
        if self.sensitive {
            return;
        }
        self.fingerprint_observations += 1;
        if !self.fingerprint_hashes.contains(fingerprint_hash) {
            if !self.fingerprint_hashes.is_empty() {
                self.fingerprint_sensitive = true;
                self.fingerprint_mismatches += 1;
            }
            if self.fingerprint_hashes.len() < QUERY_VALUE_SAMPLE_LIMIT {
                self.fingerprint_hashes.insert(fingerprint_hash.to_string());
            }
        }
        let Some(value_hash) = param.value_hash.as_ref() else {
            return;
        };
        if let Some(previous) = self.fingerprints_by_value.get(value_hash) {
            if previous != fingerprint_hash {
                self.fingerprint_sensitive = true;
                self.fingerprint_mismatches += 1;
            }
        } else if self.fingerprints_by_value.len() < QUERY_VALUE_SAMPLE_LIMIT {
            self.fingerprints_by_value
                .insert(value_hash.clone(), fingerprint_hash.to_string());
        }
        if self.value_count() > 1 && self.fingerprint_hashes.len() > 1 {
            self.fingerprint_sensitive = true;
        }
    }

    fn value_count(&self) -> usize {
        self.value_hashes.len()
            + usize::from(self.value_hash_overflow && self.value_hashes.len() < usize::MAX)
    }

    fn cardinality(&self) -> &'static str {
        if self.sensitive || (self.value_hashes.is_empty() && !self.value_hash_overflow) {
            "unknown"
        } else if self.value_hash_overflow || self.value_hashes.len() > 16 {
            "high"
        } else if self.value_hashes.len() > 4 {
            "medium"
        } else if self.value_hashes.len() > 1 {
            "low"
        } else {
            "one"
        }
    }

    pub(crate) fn suggestion(&self) -> Option<String> {
        if self.configured_action != "observe"
            || self.sensitive
            || self.fingerprint_sensitive
            || self.fingerprint_observations < QUERY_SUGGESTION_MIN_FINGERPRINTS
            || self.fingerprint_hashes.len() > 1
        {
            return None;
        }
        let known_noise = self.name.starts_with("utm_")
            || matches!(self.name.as_str(), "gclid" | "fbclid" | "mc_cid" | "mc_eid");
        let fragmented = matches!(self.cardinality(), "medium" | "high");
        if known_noise || fragmented {
            Some("candidate_ignore".to_string())
        } else {
            None
        }
    }

    pub(crate) fn verified_ignore_candidate(&self, config: &QueryEquivalenceConfig) -> bool {
        config.enabled
            && !self.sensitive
            && !self.fingerprint_sensitive
            && self.value_count() as u64 >= config.min_distinct_values
            && self.fingerprint_observations >= config.min_matching_fingerprints
            && self.fingerprint_mismatches <= config.max_mismatches
            && self.fingerprint_hashes.len() == 1
    }

    pub(crate) fn equivalence_class(
        &self,
        config: &QueryEquivalenceConfig,
        operator_enabled: bool,
    ) -> QueryEquivalenceClass {
        if self.sensitive {
            QueryEquivalenceClass::SensitiveBlocked
        } else if self.fingerprint_mismatches > config.max_mismatches {
            QueryEquivalenceClass::MismatchCooldown
        } else if self.verified_ignore_candidate(config) && operator_enabled {
            QueryEquivalenceClass::Compacted
        } else if self.verified_ignore_candidate(config) {
            QueryEquivalenceClass::VerifiedIgnoreCandidate
        } else if self.suggestion().is_some() {
            QueryEquivalenceClass::CandidateIgnore
        } else {
            QueryEquivalenceClass::Unknown
        }
    }

    pub(crate) fn snapshot(
        &self,
        config: &QueryEquivalenceConfig,
        operator_enabled: bool,
    ) -> QueryParamSnapshot {
        let equivalence_class = self.equivalence_class(config, operator_enabled);
        QueryParamSnapshot {
            name: self.name.clone(),
            seen_count: self.seen_count,
            cardinality: self.cardinality().to_string(),
            fingerprint_sensitive: self.fingerprint_sensitive,
            configured_action: self.configured_action.clone(),
            suggestion: self.suggestion(),
            equivalence_class,
            sensitive: self.sensitive,
            distinct_value_count: self.value_count() as u64,
            matching_fingerprint_count: self.fingerprint_observations,
            mismatch_count: self.fingerprint_mismatches,
            operator_enabled,
        }
    }
}

pub(crate) fn response_fingerprint_hash(fingerprint: &ResponseFingerprint) -> String {
    format!(
        "{}:{}:{}",
        fingerprint.status,
        fingerprint.header_hash,
        fingerprint.body_hash.as_deref().unwrap_or("")
    )
}
