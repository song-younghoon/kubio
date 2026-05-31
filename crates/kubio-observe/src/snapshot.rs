use kubio_core::{
    AdaptiveReuseBlocker, ConfidenceTier, DecisionReason, HeaderEquivalenceClass,
    HeaderEquivalenceSource, LatencySnapshot, QueryEquivalenceClass, ReloadStatus, ReuseClass,
    RouteId, RouteReloadSnapshot, RouteState, StatusClassCounts,
};
use serde::{Deserialize, Serialize};

use crate::events::Event;
use crate::latency::percentile;
use crate::protocol::{AltSvcCounts, Http3ServerCounts, ProtocolCounts, UpstreamHttp3Counts};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverSnapshot {
    pub overview: OverviewSnapshot,
    pub routes: Vec<RouteSnapshot>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OverviewSnapshot {
    pub observed_requests: u64,
    pub origin_requests: u64,
    pub reused_responses: u64,
    pub protected_requests: u64,
    pub bypassed_requests: u64,
    pub candidate_routes: u64,
    pub auto_routes: u64,
    pub estimated_savings: f64,
    pub actual_reuse_rate: f64,
    pub shadow_matches: u64,
    pub shadow_mismatches: u64,
    pub revalidation_attempts: u64,
    pub revalidation_not_modified: u64,
    pub revalidation_modified: u64,
    pub revalidation_failed: u64,
    pub stale_responses_served: u64,
    pub stale_responses_denied: u64,
    pub route_hints_applied: u64,
    pub route_hints_rejected: u64,
    pub query_hints_applied: u64,
    pub query_hints_rejected: u64,
    pub query_param_suggestions: u64,
    pub store_errors: u64,
    pub dropped_events: u64,
    pub backpressure_rejections: u64,
    pub protocol_fallbacks: u64,
    pub in_flight_requests: u64,
    pub max_in_flight_requests: u64,
    pub downstream_http1_requests: u64,
    pub downstream_http2_requests: u64,
    pub downstream_http3_requests: u64,
    pub upstream_http1_requests: u64,
    pub upstream_http2_requests: u64,
    pub upstream_http3_requests: u64,
    pub alt_svc: AltSvcCounts,
    pub http3_server: Http3ServerCounts,
    pub upstream_http3: UpstreamHttp3Counts,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub config_generation: u64,
    pub config_reload_attempts: ConfigReloadStatusCounts,
    pub config_reload_reloadable_changes: u64,
    pub config_reload_restart_required_changes: u64,
    pub config_reload_routes_added: u64,
    pub config_reload_routes_changed: u64,
    pub config_reload_routes_removed: u64,
    pub config_reload_routes_demoted: u64,
    pub config_reload_cache_entries_purged: u64,
}

impl OverviewSnapshot {
    pub(crate) fn from_routes(routes: &[RouteSnapshot]) -> Self {
        let mut overview = Self::default();
        let mut latencies = Vec::new();
        for route in routes {
            overview.observed_requests += route.request_count;
            overview.origin_requests += route.origin_count;
            overview.reused_responses += route.reuse_count;
            overview.protected_requests += route.protected_count;
            overview.bypassed_requests += route.bypass_count;
            overview.shadow_matches += route.shadow_matches;
            overview.shadow_mismatches += route.shadow_mismatches;
            overview.revalidation_attempts += route.revalidation_attempts;
            overview.revalidation_not_modified += route.revalidation_not_modified;
            overview.revalidation_modified += route.revalidation_modified;
            overview.revalidation_failed += route.revalidation_failed;
            overview.stale_responses_served += route.stale_served;
            overview.stale_responses_denied += route.stale_denied;
            overview.route_hints_applied += route.route_hint_applied;
            overview.route_hints_rejected += route.route_hint_rejected;
            overview.query_hints_applied += route.query_hint_applied;
            overview.query_hints_rejected += route.query_hint_rejected;
            overview.query_param_suggestions += route.query_param_suggestions;
            if route.state == RouteState::Candidate || route.state == RouteState::ShadowValidated {
                overview.candidate_routes += 1;
            }
            if route.state == RouteState::Auto {
                overview.auto_routes += 1;
            }
            latencies.push(route.latency.p50_ms);
            latencies.push(route.latency.p95_ms);
        }
        if overview.observed_requests > 0 {
            overview.estimated_savings = routes
                .iter()
                .map(|route| route.estimated_savings * route.request_count as f64)
                .sum::<f64>()
                / overview.observed_requests as f64;
            overview.actual_reuse_rate =
                overview.reused_responses as f64 / overview.observed_requests as f64;
        }
        if !latencies.is_empty() {
            latencies.sort_by(|left, right| left.total_cmp(right));
            overview.p50_latency_ms = percentile(&latencies, 0.50);
            overview.p95_latency_ms = percentile(&latencies, 0.95);
        }
        overview
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigReloadStatusCounts {
    pub applied: u64,
    pub dry_run_ok: u64,
    pub parse_failed: u64,
    pub validation_failed: u64,
    pub restart_required: u64,
    pub state_reconciliation_failed: u64,
    pub no_config_source: u64,
    pub unauthorized: u64,
    pub internal_error: u64,
}

impl ConfigReloadStatusCounts {
    pub(crate) fn increment(&mut self, status: ReloadStatus) {
        match status {
            ReloadStatus::Applied => self.applied += 1,
            ReloadStatus::DryRunOk => self.dry_run_ok += 1,
            ReloadStatus::ParseFailed => self.parse_failed += 1,
            ReloadStatus::ValidationFailed => self.validation_failed += 1,
            ReloadStatus::RestartRequired => self.restart_required += 1,
            ReloadStatus::StateReconciliationFailed => self.state_reconciliation_failed += 1,
            ReloadStatus::NoConfigSource => self.no_config_source += 1,
            ReloadStatus::Unauthorized => self.unauthorized += 1,
            ReloadStatus::InternalError => self.internal_error += 1,
        }
    }

    pub fn iter(&self) -> [(ReloadStatus, u64); 9] {
        [
            (ReloadStatus::Applied, self.applied),
            (ReloadStatus::DryRunOk, self.dry_run_ok),
            (ReloadStatus::ParseFailed, self.parse_failed),
            (ReloadStatus::ValidationFailed, self.validation_failed),
            (ReloadStatus::RestartRequired, self.restart_required),
            (
                ReloadStatus::StateReconciliationFailed,
                self.state_reconciliation_failed,
            ),
            (ReloadStatus::NoConfigSource, self.no_config_source),
            (ReloadStatus::Unauthorized, self.unauthorized),
            (ReloadStatus::InternalError, self.internal_error),
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSnapshot {
    pub route_id: RouteId,
    pub route_hash: String,
    pub state: RouteState,
    pub reuse_class: ReuseClass,
    pub request_count: u64,
    pub origin_count: u64,
    pub reuse_count: u64,
    pub protected_count: u64,
    pub bypass_count: u64,
    pub store_safe_count: u64,
    pub origin_public_responses: u64,
    pub distinct_key_count: u64,
    pub dynamic_value_count: u64,
    pub slug_value_count: u64,
    pub store_safe_rate: f64,
    pub adaptive_blockers: Vec<AdaptiveReuseBlocker>,
    pub confidence_tier: ConfidenceTier,
    pub evidence_window_age_seconds: u64,
    pub stale_evidence: bool,
    pub cooldown_remaining_seconds: Option<u64>,
    pub canary_matches: u64,
    pub canary_mismatches: u64,
    pub query_equivalence_candidates: u64,
    pub query_compacted_groups: u64,
    pub ignored_response_header_count: u64,
    pub suppressed_on_hit_header_count: u64,
    pub verified_header_ignore_candidates: u64,
    pub variant_dimensions: u64,
    pub variant_unbounded: bool,
    pub shadow_matches: u64,
    pub shadow_mismatches: u64,
    pub revalidation_attempts: u64,
    pub revalidation_not_modified: u64,
    pub revalidation_modified: u64,
    pub revalidation_failed: u64,
    pub stale_served: u64,
    pub stale_denied: u64,
    pub route_hint_applied: u64,
    pub route_hint_rejected: u64,
    pub query_hint_applied: u64,
    pub query_hint_rejected: u64,
    pub query_param_suggestions: u64,
    pub downstream_protocols: ProtocolCounts,
    pub upstream_protocols: ProtocolCounts,
    pub status_classes: StatusClassCounts,
    pub latency: LatencySnapshot,
    pub repeat_rate: f64,
    pub estimated_savings: f64,
    pub actual_reuse_rate: f64,
    pub score: i16,
    pub reasons: Vec<DecisionReason>,
    pub explanation: Vec<String>,
    pub route_hint: Option<String>,
    pub query_params: Vec<QueryParamSnapshot>,
    pub response_headers: Vec<ResponseHeaderSnapshot>,
    pub reload: RouteReloadSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParamSnapshot {
    pub name: String,
    pub seen_count: u64,
    pub cardinality: String,
    pub fingerprint_sensitive: bool,
    pub configured_action: String,
    pub suggestion: Option<String>,
    pub equivalence_class: QueryEquivalenceClass,
    pub sensitive: bool,
    pub distinct_value_count: u64,
    pub matching_fingerprint_count: u64,
    pub mismatch_count: u64,
    pub operator_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseHeaderSnapshot {
    pub name: String,
    pub class: HeaderEquivalenceClass,
    pub source: HeaderEquivalenceSource,
    pub distinct_value_count: u64,
    pub matching_without_header_count: u64,
    pub mismatch_count: u64,
    pub operator_enabled: bool,
    pub suppressed_on_hit: bool,
    pub sensitive: bool,
}

pub(crate) fn state_sort_key(state: RouteState) -> u8 {
    match state {
        RouteState::Auto => 4,
        RouteState::Candidate | RouteState::ShadowValidated => 3,
        RouteState::Protected => 2,
        RouteState::Watching => 1,
    }
}
