use kubio_core::{CacheKeyHash, DecisionReason, RouteId, RouteState, StatusClassCounts};
use std::collections::{HashMap, VecDeque};
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

    pub(crate) fn snapshot(&self) -> RouteSnapshot {
        RouteSnapshot {
            route_id: self.route_id.clone(),
            route_hash: self.route_id.hash(),
            state: self.state,
            request_count: self.request_count,
            origin_count: self.origin_count,
            reuse_count: self.reuse_count,
            protected_count: self.protected_count,
            bypass_count: self.bypass_count,
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
