use kubio_core::{
    AdaptiveReuseBlocker, AdaptiveReuseConfig, CacheKeyHash, ConfidenceTier, DecisionReason,
    HeaderEquivalenceSource, ResponseHeaderEquivalenceConfig, ReuseClass, RouteId,
    RouteReloadAction, RouteReloadSnapshot, RouteResponseHeadersConfig, RouteState,
    StatusClassCounts,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, SystemTime};

use crate::events::Event;
use crate::latency::latency_snapshot;
use crate::protocol::{AltSvcCounts, Http3ServerCounts, ProtocolCounts, UpstreamHttp3Counts};
use crate::query::QueryParamStats;
use crate::records::KeyObservation;
use crate::response_headers::ResponseHeaderStats;
use crate::snapshot::{ConfigReloadStatusCounts, RouteSnapshot};

#[derive(Debug, Clone, Default)]
pub(crate) struct ObserverInner {
    pub(crate) routes: HashMap<RouteId, RouteStats>,
    pub(crate) keys: HashMap<CacheKeyHash, KeyObservation>,
    pub(crate) events: VecDeque<Event>,
    pub(crate) store_errors: u64,
    pub(crate) dropped_events: u64,
    pub(crate) backpressure_rejections: u64,
    pub(crate) protocol_fallbacks: u64,
    pub(crate) in_flight_requests: u64,
    pub(crate) max_in_flight_requests: u64,
    pub(crate) downstream_protocols: ProtocolCounts,
    pub(crate) upstream_protocols: ProtocolCounts,
    pub(crate) alt_svc: AltSvcCounts,
    pub(crate) http3_server: Http3ServerCounts,
    pub(crate) upstream_http3: UpstreamHttp3Counts,
    pub(crate) config_generation: u64,
    pub(crate) config_reload_attempts: ConfigReloadStatusCounts,
    pub(crate) config_reload_reloadable_changes: u64,
    pub(crate) config_reload_restart_required_changes: u64,
    pub(crate) config_reload_routes_added: u64,
    pub(crate) config_reload_routes_changed: u64,
    pub(crate) config_reload_routes_removed: u64,
    pub(crate) config_reload_routes_demoted: u64,
    pub(crate) config_reload_cache_entries_purged: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RouteStats {
    pub(crate) route_id: RouteId,
    pub(crate) state: RouteState,
    pub(crate) request_count: u64,
    pub(crate) origin_count: u64,
    pub(crate) reuse_count: u64,
    pub(crate) protected_count: u64,
    pub(crate) bypass_count: u64,
    pub(crate) shadow_matches: u64,
    pub(crate) shadow_mismatches: u64,
    pub(crate) revalidation_attempts: u64,
    pub(crate) revalidation_not_modified: u64,
    pub(crate) revalidation_modified: u64,
    pub(crate) revalidation_failed: u64,
    pub(crate) stale_served: u64,
    pub(crate) stale_denied: u64,
    pub(crate) store_safe_count: u64,
    pub(crate) origin_public_responses: u64,
    pub(crate) distinct_key_hashes: HashSet<CacheKeyHash>,
    pub(crate) dynamic_segment_hashes: HashSet<String>,
    pub(crate) slug_segment_hashes: HashSet<String>,
    pub(crate) id_like_path_samples: u64,
    pub(crate) slug_like_path_samples: u64,
    pub(crate) path_sensitive_samples: u64,
    pub(crate) precision_positive_samples: u64,
    pub(crate) precision_negative_samples: u64,
    pub(crate) canary_matches: u64,
    pub(crate) canary_mismatches: u64,
    pub(crate) first_evidence_at: Option<SystemTime>,
    pub(crate) last_evidence_at: Option<SystemTime>,
    pub(crate) cooldown_until: Option<SystemTime>,
    pub(crate) cooldown_count: u64,
    pub(crate) last_canary_at: Option<SystemTime>,
    pub(crate) query_compacted_groups: HashSet<String>,
    pub(crate) variant_value_hashes: HashMap<String, HashSet<String>>,
    pub(crate) route_hint: Option<String>,
    pub(crate) route_hint_applied: u64,
    pub(crate) route_hint_rejected: u64,
    pub(crate) query_hint_applied: u64,
    pub(crate) query_hint_rejected: u64,
    pub(crate) query_param_suggestions: u64,
    pub(crate) downstream_protocols: ProtocolCounts,
    pub(crate) upstream_protocols: ProtocolCounts,
    pub(crate) status_classes: StatusClassCounts,
    pub(crate) latencies: VecDeque<Duration>,
    pub(crate) score: i16,
    pub(crate) reasons: Vec<DecisionReason>,
    pub(crate) query_params: HashMap<String, QueryParamStats>,
    pub(crate) response_headers: HashMap<String, ResponseHeaderStats>,
    pub(crate) reload: RouteReloadSnapshot,
}

impl RouteStats {
    pub(crate) fn new(route_id: RouteId) -> Self {
        Self {
            route_id,
            state: RouteState::Watching,
            request_count: 0,
            origin_count: 0,
            reuse_count: 0,
            protected_count: 0,
            bypass_count: 0,
            shadow_matches: 0,
            shadow_mismatches: 0,
            revalidation_attempts: 0,
            revalidation_not_modified: 0,
            revalidation_modified: 0,
            revalidation_failed: 0,
            stale_served: 0,
            stale_denied: 0,
            store_safe_count: 0,
            origin_public_responses: 0,
            distinct_key_hashes: HashSet::new(),
            dynamic_segment_hashes: HashSet::new(),
            slug_segment_hashes: HashSet::new(),
            id_like_path_samples: 0,
            slug_like_path_samples: 0,
            path_sensitive_samples: 0,
            precision_positive_samples: 0,
            precision_negative_samples: 0,
            canary_matches: 0,
            canary_mismatches: 0,
            first_evidence_at: None,
            last_evidence_at: None,
            cooldown_until: None,
            cooldown_count: 0,
            last_canary_at: None,
            query_compacted_groups: HashSet::new(),
            variant_value_hashes: HashMap::new(),
            route_hint: None,
            route_hint_applied: 0,
            route_hint_rejected: 0,
            query_hint_applied: 0,
            query_hint_rejected: 0,
            query_param_suggestions: 0,
            downstream_protocols: ProtocolCounts::default(),
            upstream_protocols: ProtocolCounts::default(),
            status_classes: StatusClassCounts::default(),
            latencies: VecDeque::new(),
            score: 0,
            reasons: Vec::new(),
            query_params: HashMap::new(),
            response_headers: HashMap::new(),
            reload: RouteReloadSnapshot::default(),
        }
    }

    pub(crate) fn repeat_rate(&self) -> f64 {
        if self.request_count == 0 {
            0.0
        } else {
            self.shadow_matches as f64 / self.request_count as f64
        }
    }

    pub(crate) fn estimated_savings(&self) -> f64 {
        if self.request_count == 0 {
            0.0
        } else {
            let reusable = self
                .request_count
                .saturating_sub(self.protected_count)
                .saturating_sub(self.bypass_count);
            reusable as f64 / self.request_count as f64
        }
    }

    pub(crate) fn store_safe_rate(&self) -> f64 {
        if self.origin_count == 0 {
            0.0
        } else {
            self.store_safe_count as f64 / self.origin_count as f64
        }
    }

    pub(crate) fn distinct_key_count(&self) -> u64 {
        self.distinct_key_hashes.len() as u64
    }

    pub(crate) fn dynamic_value_count(&self) -> u64 {
        (self.dynamic_segment_hashes.len() + self.slug_segment_hashes.len()) as u64
    }

    pub(crate) fn slug_value_count(&self) -> u64 {
        self.slug_segment_hashes.len() as u64
    }

    pub(crate) fn evidence_age_seconds(&self) -> u64 {
        self.last_evidence_at
            .and_then(|last| SystemTime::now().duration_since(last).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    }

    pub(crate) fn stale_evidence(&self, config: &AdaptiveReuseConfig) -> bool {
        config.precision.enabled
            && self
                .last_evidence_at
                .and_then(|last| SystemTime::now().duration_since(last).ok())
                .map(|age| age.as_secs() >= config.precision.confidence.fresh_window_secs)
                .unwrap_or(false)
    }

    pub(crate) fn cooldown_remaining_seconds(&self) -> Option<u64> {
        self.cooldown_until
            .and_then(|until| until.duration_since(SystemTime::now()).ok())
            .map(|duration| duration.as_secs())
            .filter(|remaining| *remaining > 0)
    }

    pub(crate) fn confidence_tier(&self, config: &AdaptiveReuseConfig) -> ConfidenceTier {
        if self.cooldown_remaining_seconds().is_some() {
            return ConfidenceTier::Cooldown;
        }
        if self.state == RouteState::Protected {
            return ConfidenceTier::HardProtected;
        }
        if !config.precision.enabled {
            return ConfidenceTier::Unknown;
        }
        if self.stale_evidence(config) {
            return ConfidenceTier::Unknown;
        }
        if self.precision_negative_samples > config.precision.confidence.max_negative_events {
            return ConfidenceTier::Cooldown;
        }
        if self.precision_positive_samples >= config.precision.confidence.strong_window_samples {
            ConfidenceTier::Strong
        } else if self.precision_positive_samples >= config.precision.confidence.min_window_samples
        {
            ConfidenceTier::Validated
        } else if self.public_object_candidate(config) || self.precision_positive_samples > 0 {
            ConfidenceTier::Probation
        } else {
            ConfidenceTier::Unknown
        }
    }

    pub(crate) fn variant_dimension_count(&self) -> u64 {
        self.variant_value_hashes.len() as u64
    }

    pub(crate) fn variant_unbounded(&self, config: &AdaptiveReuseConfig) -> bool {
        self.variant_value_hashes
            .values()
            .any(|values| values.len() as u64 > config.precision.variants.max_values_per_dimension)
    }

    pub(crate) fn public_object_ready(&self, config: &AdaptiveReuseConfig) -> bool {
        let public_object = &config.public_object;
        config.enabled
            && public_object.enabled
            && self.state != RouteState::Protected
            && self.shadow_mismatches <= public_object.max_shadow_mismatches
            && self.request_count >= public_object.min_route_samples
            && self.distinct_key_count() >= public_object.min_distinct_keys
            && self.dynamic_value_count() >= public_object.min_distinct_keys
            && self.store_safe_rate() >= public_object.min_store_safe_rate
            && self.shadow_matches >= public_object.min_shadow_matches
            && self.path_sensitive_samples == 0
            && self.cooldown_remaining_seconds().is_none()
            && !self.stale_evidence(config)
    }

    pub(crate) fn public_object_candidate(&self, config: &AdaptiveReuseConfig) -> bool {
        config.enabled
            && config.public_object.enabled
            && self.state != RouteState::Protected
            && self.path_sensitive_samples == 0
            && (self.id_like_path_samples > 0
                || self.slug_like_path_samples > 0
                || self.distinct_key_count() > 1)
            && self.shadow_mismatches <= config.public_object.max_shadow_mismatches
            && self.cooldown_remaining_seconds().is_none()
    }

    pub(crate) fn reuse_class(&self, config: &AdaptiveReuseConfig) -> ReuseClass {
        if self.state == RouteState::Protected {
            ReuseClass::HardProtected
        } else if config.enabled
            && config.origin_public_fast_path.enabled
            && self.origin_public_responses > 0
        {
            ReuseClass::OriginPublic
        } else if self.public_object_ready(config) {
            ReuseClass::PublicObject
        } else if self.public_object_candidate(config) {
            ReuseClass::PublicObjectCandidate
        } else if self.state == RouteState::Auto {
            ReuseClass::KeyValidated
        } else {
            ReuseClass::Watching
        }
    }

    pub(crate) fn eligibility_blockers(
        &self,
        config: &AdaptiveReuseConfig,
    ) -> Vec<AdaptiveReuseBlocker> {
        let mut blockers = Vec::new();
        if !config.enabled {
            blockers.push(AdaptiveReuseBlocker::Disabled);
        }
        if self.state == RouteState::Protected {
            blockers.push(AdaptiveReuseBlocker::ProtectedRoute);
        }
        if self.shadow_mismatches > config.public_object.max_shadow_mismatches {
            blockers.push(AdaptiveReuseBlocker::ShadowMismatch);
        }
        if self.cooldown_remaining_seconds().is_some() {
            blockers.push(AdaptiveReuseBlocker::CooldownActive);
        }
        if self.stale_evidence(config) {
            blockers.push(AdaptiveReuseBlocker::StaleEvidence);
        }
        if self.request_count < config.public_object.min_route_samples {
            blockers.push(AdaptiveReuseBlocker::InsufficientRouteSamples);
        }
        if self.distinct_key_count() < config.public_object.min_distinct_keys {
            blockers.push(AdaptiveReuseBlocker::InsufficientDistinctKeys);
        }
        if self.dynamic_value_count() < config.public_object.min_distinct_keys {
            blockers.push(AdaptiveReuseBlocker::LowPathCardinality);
        }
        if self.store_safe_rate() < config.public_object.min_store_safe_rate {
            blockers.push(AdaptiveReuseBlocker::LowStoreSafeRate);
        }
        if config.origin_public_fast_path.enabled && self.origin_public_responses == 0 {
            blockers.push(AdaptiveReuseBlocker::NoOriginPublicSignal);
        }
        if config.precision.enabled && self.variant_unbounded(config) {
            blockers.push(AdaptiveReuseBlocker::VariantUnbounded);
        }
        blockers
    }

    pub(crate) fn snapshot(
        &self,
        adaptive_config: &AdaptiveReuseConfig,
        header_config: &ResponseHeaderEquivalenceConfig,
        route_response_headers: Option<&RouteResponseHeadersConfig>,
    ) -> RouteSnapshot {
        let response_headers = self
            .response_headers
            .values()
            .map(|stats| {
                let operator_enabled = route_response_headers
                    .map(|headers| {
                        headers.verified_ignore.enabled
                            && headers.verified_ignore.allow.iter().any(|pattern| {
                                kubio_core::response_header_pattern_matches(pattern, &stats.name)
                            })
                    })
                    .unwrap_or(false)
                    || (stats.source == HeaderEquivalenceSource::RouteHint
                        && stats.ignored_count > 0);
                let force_included = route_response_headers
                    .map(|headers| {
                        headers.force_include.iter().any(|pattern| {
                            kubio_core::response_header_pattern_matches(pattern, &stats.name)
                        })
                    })
                    .unwrap_or(false)
                    || stats.source == HeaderEquivalenceSource::ForceInclude
                    || header_config.default_volatile.block.iter().any(|pattern| {
                        kubio_core::response_header_pattern_matches(pattern, &stats.name)
                    });
                stats.snapshot(header_config, operator_enabled, force_included)
            })
            .collect::<Vec<_>>();
        let verified_header_ignore_candidates = self
            .response_headers
            .values()
            .filter(|stats| stats.verified_candidate(header_config))
            .count() as u64;
        let ignored_response_header_count = self
            .response_headers
            .values()
            .map(|stats| stats.default_ignored_count + stats.ignored_count)
            .sum::<u64>();
        let suppressed_on_hit_header_count = self
            .response_headers
            .values()
            .map(|stats| stats.suppressed_on_hit_count)
            .sum::<u64>();
        RouteSnapshot {
            route_id: self.route_id.clone(),
            route_hash: self.route_id.hash(),
            state: self.state,
            reuse_class: self.reuse_class(adaptive_config),
            request_count: self.request_count,
            origin_count: self.origin_count,
            reuse_count: self.reuse_count,
            protected_count: self.protected_count,
            bypass_count: self.bypass_count,
            store_safe_count: self.store_safe_count,
            origin_public_responses: self.origin_public_responses,
            distinct_key_count: self.distinct_key_count(),
            dynamic_value_count: self.dynamic_value_count(),
            slug_value_count: self.slug_value_count(),
            store_safe_rate: self.store_safe_rate(),
            adaptive_blockers: self.eligibility_blockers(adaptive_config),
            confidence_tier: self.confidence_tier(adaptive_config),
            evidence_window_age_seconds: self.evidence_age_seconds(),
            stale_evidence: self.stale_evidence(adaptive_config),
            cooldown_remaining_seconds: self.cooldown_remaining_seconds(),
            canary_matches: self.canary_matches,
            canary_mismatches: self.canary_mismatches,
            query_equivalence_candidates: self
                .query_params
                .values()
                .filter(|stats| {
                    stats.verified_ignore_candidate(&adaptive_config.precision.query_equivalence)
                })
                .count() as u64,
            query_compacted_groups: self.query_compacted_groups.len() as u64,
            ignored_response_header_count,
            suppressed_on_hit_header_count,
            verified_header_ignore_candidates,
            variant_dimensions: self.variant_dimension_count(),
            variant_unbounded: self.variant_unbounded(adaptive_config),
            shadow_matches: self.shadow_matches,
            shadow_mismatches: self.shadow_mismatches,
            revalidation_attempts: self.revalidation_attempts,
            revalidation_not_modified: self.revalidation_not_modified,
            revalidation_modified: self.revalidation_modified,
            revalidation_failed: self.revalidation_failed,
            stale_served: self.stale_served,
            stale_denied: self.stale_denied,
            route_hint_applied: self.route_hint_applied,
            route_hint_rejected: self.route_hint_rejected,
            query_hint_applied: self.query_hint_applied,
            query_hint_rejected: self.query_hint_rejected,
            query_param_suggestions: self.query_param_suggestions,
            downstream_protocols: self.downstream_protocols.clone(),
            upstream_protocols: self.upstream_protocols.clone(),
            status_classes: self.status_classes.clone(),
            latency: latency_snapshot(&self.latencies),
            repeat_rate: self.repeat_rate(),
            estimated_savings: self.estimated_savings(),
            actual_reuse_rate: if self.request_count == 0 {
                0.0
            } else {
                self.reuse_count as f64 / self.request_count as f64
            },
            score: self.score,
            reasons: self.reasons.clone(),
            explanation: self
                .reasons
                .iter()
                .map(|reason| reason.user_message().to_string())
                .collect(),
            route_hint: self.route_hint.clone(),
            query_params: self
                .query_params
                .values()
                .map(|stats| {
                    stats.snapshot(
                        &adaptive_config.precision.query_equivalence,
                        self.query_compacted_groups.contains(&stats.name),
                    )
                })
                .collect(),
            response_headers,
            reload: self.reload.clone(),
        }
    }

    pub(crate) fn demote_for_reload(
        &mut self,
        generation: u64,
        action: RouteReloadAction,
        reason: &str,
    ) {
        if self.state != RouteState::Protected {
            self.state = RouteState::Watching;
        }
        self.shadow_matches = 0;
        self.origin_public_responses = 0;
        self.precision_positive_samples = 0;
        self.canary_matches = 0;
        self.query_compacted_groups.clear();
        self.response_headers.clear();
        self.reload = RouteReloadSnapshot {
            last_config_generation: generation,
            last_reload_action: Some(action),
            last_reload_reason: Some(reason.to_string()),
        };
    }
}
