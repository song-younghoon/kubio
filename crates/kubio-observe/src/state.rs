use kubio_core::{
    AdaptiveReuseBlocker, AdaptiveReuseConfig, CacheKeyHash, DecisionReason, ReuseClass, RouteId,
    RouteState, StatusClassCounts,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use crate::events::Event;
use crate::latency::latency_snapshot;
use crate::protocol::{AltSvcCounts, Http3ServerCounts, ProtocolCounts, UpstreamHttp3Counts};
use crate::query::QueryParamStats;
use crate::records::KeyObservation;
use crate::snapshot::RouteSnapshot;

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
    pub(crate) id_like_path_samples: u64,
    pub(crate) path_sensitive_samples: u64,
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
            id_like_path_samples: 0,
            path_sensitive_samples: 0,
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
        self.dynamic_segment_hashes.len() as u64
    }

    pub(crate) fn public_object_ready(&self, config: &AdaptiveReuseConfig) -> bool {
        let public_object = &config.public_object;
        config.enabled
            && public_object.enabled
            && self.state != RouteState::Protected
            && (self.shadow_mismatches as u64) <= public_object.max_shadow_mismatches
            && self.request_count >= public_object.min_route_samples
            && self.distinct_key_count() >= public_object.min_distinct_keys
            && self.dynamic_value_count() >= public_object.min_distinct_keys
            && self.store_safe_rate() >= public_object.min_store_safe_rate
            && self.shadow_matches >= public_object.min_shadow_matches
            && self.path_sensitive_samples == 0
    }

    pub(crate) fn public_object_candidate(&self, config: &AdaptiveReuseConfig) -> bool {
        config.enabled
            && config.public_object.enabled
            && self.state != RouteState::Protected
            && self.path_sensitive_samples == 0
            && (self.id_like_path_samples > 0 || self.distinct_key_count() > 1)
            && (self.shadow_mismatches as u64) <= config.public_object.max_shadow_mismatches
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
        if (self.shadow_mismatches as u64) > config.public_object.max_shadow_mismatches {
            blockers.push(AdaptiveReuseBlocker::ShadowMismatch);
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
        blockers
    }

    pub(crate) fn snapshot(&self, config: &AdaptiveReuseConfig) -> RouteSnapshot {
        RouteSnapshot {
            route_id: self.route_id.clone(),
            route_hash: self.route_id.hash(),
            state: self.state,
            reuse_class: self.reuse_class(config),
            request_count: self.request_count,
            origin_count: self.origin_count,
            reuse_count: self.reuse_count,
            protected_count: self.protected_count,
            bypass_count: self.bypass_count,
            store_safe_count: self.store_safe_count,
            origin_public_responses: self.origin_public_responses,
            distinct_key_count: self.distinct_key_count(),
            dynamic_value_count: self.dynamic_value_count(),
            store_safe_rate: self.store_safe_rate(),
            adaptive_blockers: self.eligibility_blockers(config),
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
                .map(QueryParamStats::snapshot)
                .collect(),
        }
    }
}
