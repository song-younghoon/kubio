use kubio_core::RouteId;
use kubio_observe::{AltSvcCounts, Http3ServerCounts, UpstreamHttp3Counts};
use kubio_store::{PurgeResult, StoreStats};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct OverviewResponse {
    pub mode: String,
    pub origin: String,
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
    pub cache_entries: u64,
    pub cache_bytes: u64,
    pub store_kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PurgeRequest {
    pub selector: String,
    pub route_id: Option<String>,
    pub cache_key_hash: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub(crate) struct StoreView {
    pub(crate) stats: StoreStats,
    pub(crate) last_purge: Option<PurgeResult>,
}

pub(crate) fn parse_route_id(value: &str) -> Option<RouteId> {
    let (method, path) = value.split_once(' ')?;
    Some(RouteId::new(method, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_id_parser_accepts_method_space_path() {
        let route = parse_route_id("GET /api/products").unwrap();
        assert_eq!(route.method, "GET");
        assert_eq!(route.template, "/api/products");
    }
}
