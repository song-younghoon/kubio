use kubio_core::{
    HeaderEquivalenceClass, HeaderEquivalenceSource, ResponseHeaderEquivalenceConfig,
    ResponseHeaderObservation,
};
use std::collections::{HashMap, HashSet};

use crate::snapshot::ResponseHeaderSnapshot;

pub(crate) const RESPONSE_HEADER_VALUE_SAMPLE_LIMIT: usize = 32;

#[derive(Debug, Clone)]
pub(crate) struct ResponseHeaderStats {
    pub(crate) name: String,
    pub(crate) value_hashes: HashSet<String>,
    pub(crate) fingerprints_by_value: HashMap<String, String>,
    pub(crate) fingerprint_hashes: HashSet<String>,
    pub(crate) observations: u64,
    pub(crate) mismatches: u64,
    pub(crate) default_ignored_count: u64,
    pub(crate) ignored_count: u64,
    pub(crate) suppressed_on_hit_count: u64,
    pub(crate) sensitive: bool,
    pub(crate) verified_event_emitted: bool,
    pub(crate) source: HeaderEquivalenceSource,
}

impl ResponseHeaderStats {
    pub(crate) fn new(name: String) -> Self {
        Self {
            name,
            value_hashes: HashSet::new(),
            fingerprints_by_value: HashMap::new(),
            fingerprint_hashes: HashSet::new(),
            observations: 0,
            mismatches: 0,
            default_ignored_count: 0,
            ignored_count: 0,
            suppressed_on_hit_count: 0,
            sensitive: false,
            verified_event_emitted: false,
            source: HeaderEquivalenceSource::VerifiedEvidence,
        }
    }

    pub(crate) fn record(&mut self, observation: &ResponseHeaderObservation) {
        self.source = observation.source;
        self.sensitive |= observation.sensitive;
        if observation.default_ignored {
            self.default_ignored_count += 1;
        }
        if observation.ignored {
            self.ignored_count += 1;
        }
        if observation.suppressed_on_hit {
            self.suppressed_on_hit_count += 1;
        }
        if self.sensitive || observation.default_ignored || observation.ignored {
            return;
        }

        self.observations += 1;
        if self.value_hashes.len() < RESPONSE_HEADER_VALUE_SAMPLE_LIMIT {
            self.value_hashes.insert(observation.value_hash.clone());
        }
        if !self
            .fingerprint_hashes
            .contains(&observation.fingerprint_hash_without_header)
        {
            if !self.fingerprint_hashes.is_empty() {
                self.mismatches += 1;
            }
            if self.fingerprint_hashes.len() < RESPONSE_HEADER_VALUE_SAMPLE_LIMIT {
                self.fingerprint_hashes
                    .insert(observation.fingerprint_hash_without_header.clone());
            }
        }
        if let Some(previous) = self.fingerprints_by_value.get(&observation.value_hash) {
            if previous != &observation.fingerprint_hash_without_header {
                self.mismatches += 1;
            }
        } else if self.fingerprints_by_value.len() < RESPONSE_HEADER_VALUE_SAMPLE_LIMIT {
            self.fingerprints_by_value.insert(
                observation.value_hash.clone(),
                observation.fingerprint_hash_without_header.clone(),
            );
        }
    }

    pub(crate) fn verified_candidate(&self, config: &ResponseHeaderEquivalenceConfig) -> bool {
        config.enabled
            && config.verified_ignore.enabled
            && !self.sensitive
            && self.value_hashes.len() as u64 >= config.verified_ignore.min_distinct_values
            && self.observations >= config.verified_ignore.min_matching_fingerprints
            && self.mismatches <= config.verified_ignore.max_mismatches
            && self.fingerprint_hashes.len() == 1
    }

    pub(crate) fn class(
        &self,
        config: &ResponseHeaderEquivalenceConfig,
        _operator_enabled: bool,
        force_included: bool,
    ) -> HeaderEquivalenceClass {
        if force_included {
            HeaderEquivalenceClass::ForceIncluded
        } else if self.sensitive {
            HeaderEquivalenceClass::SensitiveBlocked
        } else if self.mismatches > config.verified_ignore.max_mismatches {
            HeaderEquivalenceClass::MismatchCooldown
        } else if self.ignored_count > 0 {
            HeaderEquivalenceClass::Ignored
        } else if self.default_ignored_count > 0 {
            HeaderEquivalenceClass::DefaultIgnored
        } else if self.verified_candidate(config) {
            HeaderEquivalenceClass::VerifiedVolatileCandidate
        } else if self.observations > 0 {
            HeaderEquivalenceClass::CandidateVolatile
        } else {
            HeaderEquivalenceClass::Unknown
        }
    }

    pub(crate) fn snapshot(
        &self,
        config: &ResponseHeaderEquivalenceConfig,
        operator_enabled: bool,
        force_included: bool,
    ) -> ResponseHeaderSnapshot {
        ResponseHeaderSnapshot {
            name: self.name.clone(),
            class: self.class(config, operator_enabled, force_included),
            source: self.source,
            distinct_value_count: self.value_hashes.len() as u64,
            matching_without_header_count: self.observations,
            mismatch_count: self.mismatches,
            operator_enabled,
            suppressed_on_hit: self.suppressed_on_hit_count > 0,
            sensitive: self.sensitive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_stats_promote_verified_volatile_candidates() {
        let mut stats = ResponseHeaderStats::new("x-vendor-execution-id".to_string());
        for value in ["a", "b", "c"] {
            stats.record(&ResponseHeaderObservation {
                name: "x-vendor-execution-id".to_string(),
                value_hash: value.to_string(),
                fingerprint_hash_without_header: "same".to_string(),
                default_ignored: false,
                ignored: false,
                suppressed_on_hit: false,
                sensitive: false,
                source: HeaderEquivalenceSource::VerifiedEvidence,
            });
        }

        assert!(stats.verified_candidate(&ResponseHeaderEquivalenceConfig::default()));
        assert_eq!(
            stats
                .snapshot(&ResponseHeaderEquivalenceConfig::default(), false, false)
                .class,
            HeaderEquivalenceClass::VerifiedVolatileCandidate
        );
    }

    #[test]
    fn header_stats_block_mismatched_candidates() {
        let mut stats = ResponseHeaderStats::new("x-vendor-execution-id".to_string());
        for (value, fingerprint) in [("a", "one"), ("b", "two"), ("c", "three")] {
            stats.record(&ResponseHeaderObservation {
                name: "x-vendor-execution-id".to_string(),
                value_hash: value.to_string(),
                fingerprint_hash_without_header: fingerprint.to_string(),
                default_ignored: false,
                ignored: false,
                suppressed_on_hit: false,
                sensitive: false,
                source: HeaderEquivalenceSource::VerifiedEvidence,
            });
        }

        assert!(!stats.verified_candidate(&ResponseHeaderEquivalenceConfig::default()));
        assert_eq!(
            stats
                .snapshot(&ResponseHeaderEquivalenceConfig::default(), false, false)
                .class,
            HeaderEquivalenceClass::MismatchCooldown
        );
    }
}
