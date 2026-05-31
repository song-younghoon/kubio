//! Prometheus text rendering helpers.

use kubio_core::{ConfidenceTier, Decision, ReuseClass, StatusClass};
use kubio_observe::ObserverSnapshot;
use kubio_store::StoreStats;

use crate::histogram::histogram;
use crate::labels::sanitize_label;
use crate::store::{store_kind_label, store_operation_latency, store_operation_metrics};
use crate::text::{line, metric};

pub fn render_metrics(snapshot: &ObserverSnapshot, store: &StoreStats) -> String {
    let mut out = String::new();
    line(
        &mut out,
        "kubio_requests_total",
        "Total requests observed by kubio.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_requests_total",
        &[],
        snapshot.overview.observed_requests,
    );

    line(
        &mut out,
        "kubio_origin_requests_total",
        "Total requests sent to origin.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_origin_requests_total",
        &[],
        snapshot.overview.origin_requests,
    );

    line(
        &mut out,
        "kubio_reused_responses_total",
        "Total responses reused by kubio.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_reused_responses_total",
        &[],
        snapshot.overview.reused_responses,
    );

    line(
        &mut out,
        "kubio_protected_requests_total",
        "Total protected requests.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_protected_requests_total",
        &[],
        snapshot.overview.protected_requests,
    );

    line(
        &mut out,
        "kubio_bypass_requests_total",
        "Total bypassed requests.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_bypass_requests_total",
        &[],
        snapshot.overview.bypassed_requests,
    );

    line(
        &mut out,
        "kubio_shadow_matches_total",
        "Total shadow validation matches.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_shadow_matches_total",
        &[],
        snapshot.overview.shadow_matches,
    );

    line(
        &mut out,
        "kubio_shadow_mismatches_total",
        "Total shadow validation mismatches.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_shadow_mismatches_total",
        &[],
        snapshot.overview.shadow_mismatches,
    );

    line(&mut out, "kubio_cache_entries", "Cache entries.", "gauge");
    let store_kind = store_kind_label(store.kind);
    metric(
        &mut out,
        "kubio_cache_entries",
        &[("store", store_kind)],
        store.entries,
    );
    line(&mut out, "kubio_cache_bytes", "Cache bytes.", "gauge");
    metric(
        &mut out,
        "kubio_cache_bytes",
        &[("store", store_kind)],
        store.bytes,
    );
    line(
        &mut out,
        "kubio_cache_evictions_total",
        "Cache evictions.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_cache_evictions_total",
        &[("store", store_kind)],
        store.evictions,
    );

    line(
        &mut out,
        "kubio_revalidation_attempts_total",
        "Total conditional revalidation attempts.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_revalidation_attempts_total",
        &[],
        snapshot.overview.revalidation_attempts,
    );
    line(
        &mut out,
        "kubio_revalidation_outcomes_total",
        "Conditional revalidation outcomes.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_revalidation_outcomes_total",
        &[("outcome", "not_modified")],
        snapshot.overview.revalidation_not_modified,
    );
    metric(
        &mut out,
        "kubio_revalidation_outcomes_total",
        &[("outcome", "modified")],
        snapshot.overview.revalidation_modified,
    );
    metric(
        &mut out,
        "kubio_revalidation_outcomes_total",
        &[("outcome", "failed")],
        snapshot.overview.revalidation_failed,
    );
    line(
        &mut out,
        "kubio_stale_responses_served_total",
        "Total stale responses served during origin errors.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_stale_responses_served_total",
        &[],
        snapshot.overview.stale_responses_served,
    );
    line(
        &mut out,
        "kubio_stale_responses_denied_total",
        "Total stale responses denied.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_stale_responses_denied_total",
        &[],
        snapshot.overview.stale_responses_denied,
    );
    line(
        &mut out,
        "kubio_route_hints_applied_total",
        "Total route hints applied.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_route_hints_applied_total",
        &[],
        snapshot.overview.route_hints_applied,
    );
    line(
        &mut out,
        "kubio_route_hints_rejected_total",
        "Total route hints rejected by safety policy.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_route_hints_rejected_total",
        &[],
        snapshot.overview.route_hints_rejected,
    );
    line(
        &mut out,
        "kubio_query_hints_applied_total",
        "Total query hints applied to cache keys.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_query_hints_applied_total",
        &[],
        snapshot.overview.query_hints_applied,
    );
    line(
        &mut out,
        "kubio_query_hints_rejected_total",
        "Total query hints rejected or unused.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_query_hints_rejected_total",
        &[],
        snapshot.overview.query_hints_rejected,
    );
    line(
        &mut out,
        "kubio_query_param_suggestions_total",
        "Total query parameter ignore suggestions created.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_query_param_suggestions_total",
        &[],
        snapshot.overview.query_param_suggestions,
    );
    line(
        &mut out,
        "kubio_routes_by_reuse_class",
        "Observed routes by adaptive reuse class.",
        "gauge",
    );
    for class in [
        ReuseClass::Watching,
        ReuseClass::HardProtected,
        ReuseClass::KeyValidated,
        ReuseClass::PublicObjectCandidate,
        ReuseClass::PublicObject,
        ReuseClass::OriginPublic,
        ReuseClass::QueryEquivalence,
    ] {
        let count = snapshot
            .routes
            .iter()
            .filter(|route| route.reuse_class == class)
            .count() as u64;
        metric(
            &mut out,
            "kubio_routes_by_reuse_class",
            &[("class", class.to_string().as_str())],
            count,
        );
    }
    line(
        &mut out,
        "kubio_origin_public_fast_path_total",
        "Origin responses that advertised public cacheability for adaptive reuse.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_origin_public_fast_path_total",
        &[("outcome", "observed")],
        snapshot
            .routes
            .iter()
            .map(|route| route.origin_public_responses)
            .sum::<u64>(),
    );
    line(
        &mut out,
        "kubio_precision_confidence_routes",
        "Observed routes by precision confidence tier.",
        "gauge",
    );
    for tier in [
        ConfidenceTier::Unknown,
        ConfidenceTier::Probation,
        ConfidenceTier::Validated,
        ConfidenceTier::Strong,
        ConfidenceTier::Cooldown,
        ConfidenceTier::HardProtected,
    ] {
        let count = snapshot
            .routes
            .iter()
            .filter(|route| route.confidence_tier == tier)
            .count() as u64;
        metric(
            &mut out,
            "kubio_precision_confidence_routes",
            &[("tier", tier.to_string().as_str())],
            count,
        );
    }
    line(
        &mut out,
        "kubio_precision_canary_total",
        "Precision canary validations by outcome.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_precision_canary_total",
        &[("outcome", "match")],
        snapshot
            .routes
            .iter()
            .map(|route| route.canary_matches)
            .sum::<u64>(),
    );
    metric(
        &mut out,
        "kubio_precision_canary_total",
        &[("outcome", "mismatch")],
        snapshot
            .routes
            .iter()
            .map(|route| route.canary_mismatches)
            .sum::<u64>(),
    );
    line(
        &mut out,
        "kubio_query_equivalence_candidates_total",
        "Verified query equivalence candidates.",
        "gauge",
    );
    metric(
        &mut out,
        "kubio_query_equivalence_candidates_total",
        &[("class", "verified_ignore_candidate")],
        snapshot
            .routes
            .iter()
            .map(|route| route.query_equivalence_candidates)
            .sum::<u64>(),
    );
    line(
        &mut out,
        "kubio_response_header_equivalence_candidates_total",
        "Verified response header volatile candidates.",
        "gauge",
    );
    metric(
        &mut out,
        "kubio_response_header_equivalence_candidates_total",
        &[("class", "verified_volatile_candidate")],
        snapshot
            .routes
            .iter()
            .map(|route| route.verified_header_ignore_candidates)
            .sum::<u64>(),
    );
    line(
        &mut out,
        "kubio_response_header_ignored_total",
        "Response headers ignored for fingerprint normalization.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_response_header_ignored_total",
        &[("source", "all")],
        snapshot
            .routes
            .iter()
            .map(|route| route.ignored_response_header_count)
            .sum::<u64>(),
    );
    line(
        &mut out,
        "kubio_response_header_suppressed_on_hit_total",
        "Response headers suppressed from cache-hit responses.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_response_header_suppressed_on_hit_total",
        &[("source", "all")],
        snapshot
            .routes
            .iter()
            .map(|route| route.suppressed_on_hit_header_count)
            .sum::<u64>(),
    );
    line(
        &mut out,
        "kubio_variant_groups",
        "Configured variant dimensions observed for precision reuse.",
        "gauge",
    );
    metric(
        &mut out,
        "kubio_variant_groups",
        &[("dimension_class", "bounded")],
        snapshot
            .routes
            .iter()
            .filter(|route| !route.variant_unbounded)
            .map(|route| route.variant_dimensions)
            .sum::<u64>(),
    );
    metric(
        &mut out,
        "kubio_variant_groups",
        &[("dimension_class", "unbounded")],
        snapshot
            .routes
            .iter()
            .filter(|route| route.variant_unbounded)
            .map(|route| route.variant_dimensions)
            .sum::<u64>(),
    );
    line(
        &mut out,
        "kubio_store_errors_total",
        "Total store errors or corrupt entries observed.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_store_errors_total",
        &[("store", store_kind)],
        snapshot.overview.store_errors,
    );
    line(
        &mut out,
        "kubio_observer_events_dropped_total",
        "Observation events dropped because the bounded event buffer was full.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_observer_events_dropped_total",
        &[],
        snapshot.overview.dropped_events,
    );
    line(
        &mut out,
        "kubio_store_operations_total",
        "Store operations by operation and result.",
        "counter",
    );
    store_operation_metrics(
        &mut out,
        store_kind,
        "get",
        store.operations.get.count,
        store.operations.get.error_count,
    );
    store_operation_metrics(
        &mut out,
        store_kind,
        "put",
        store.operations.put.count,
        store.operations.put.error_count,
    );
    store_operation_metrics(
        &mut out,
        store_kind,
        "purge",
        store.operations.purge.count,
        store.operations.purge.error_count,
    );
    line(
        &mut out,
        "kubio_store_operation_duration_seconds_sum",
        "Total store operation duration by operation.",
        "counter",
    );
    store_operation_latency(
        &mut out,
        store_kind,
        "get",
        store.operations.get.total_latency_us,
    );
    store_operation_latency(
        &mut out,
        store_kind,
        "put",
        store.operations.put.total_latency_us,
    );
    store_operation_latency(
        &mut out,
        store_kind,
        "purge",
        store.operations.purge.total_latency_us,
    );
    line(
        &mut out,
        "kubio_store_saturation_events_total",
        "Store operations rejected because configured store limits were reached.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_store_saturation_events_total",
        &[("store", store_kind)],
        store.operations.saturation_events,
    );
    line(
        &mut out,
        "kubio_downstream_requests_total",
        "Requests by downstream protocol.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_downstream_requests_total",
        &[("protocol", "http1")],
        snapshot.overview.downstream_http1_requests,
    );
    metric(
        &mut out,
        "kubio_downstream_requests_total",
        &[("protocol", "http2")],
        snapshot.overview.downstream_http2_requests,
    );
    metric(
        &mut out,
        "kubio_downstream_requests_total",
        &[("protocol", "http3")],
        snapshot.overview.downstream_http3_requests,
    );
    line(
        &mut out,
        "kubio_upstream_requests_total",
        "Origin requests by upstream protocol.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_upstream_requests_total",
        &[("protocol", "http1")],
        snapshot.overview.upstream_http1_requests,
    );
    metric(
        &mut out,
        "kubio_upstream_requests_total",
        &[("protocol", "http2")],
        snapshot.overview.upstream_http2_requests,
    );
    metric(
        &mut out,
        "kubio_upstream_requests_total",
        &[("protocol", "http3")],
        snapshot.overview.upstream_http3_requests,
    );
    line(
        &mut out,
        "kubio_backpressure_rejections_total",
        "Requests rejected because kubio reached a configured concurrency limit.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_backpressure_rejections_total",
        &[],
        snapshot.overview.backpressure_rejections,
    );
    line(
        &mut out,
        "kubio_in_flight_requests",
        "Current in-flight proxy requests.",
        "gauge",
    );
    metric(
        &mut out,
        "kubio_in_flight_requests",
        &[],
        snapshot.overview.in_flight_requests,
    );
    line(
        &mut out,
        "kubio_max_in_flight_requests",
        "Configured in-flight request limit.",
        "gauge",
    );
    metric(
        &mut out,
        "kubio_max_in_flight_requests",
        &[],
        snapshot.overview.max_in_flight_requests,
    );
    line(
        &mut out,
        "kubio_protocol_fallbacks_total",
        "Origin protocol fallbacks observed.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_protocol_fallbacks_total",
        &[],
        snapshot.overview.protocol_fallbacks,
    );
    line(
        &mut out,
        "kubio_alt_svc_advertisements_total",
        "Alt-Svc advertisement decisions with bounded reasons.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_alt_svc_advertisements_total",
        &[
            ("outcome", "advertised"),
            ("reason", "configured_authority"),
        ],
        snapshot.overview.alt_svc.advertised,
    );
    metric(
        &mut out,
        "kubio_alt_svc_advertisements_total",
        &[("outcome", "skipped"), ("reason", "http3_disabled")],
        snapshot.overview.alt_svc.skipped_http3_disabled,
    );
    metric(
        &mut out,
        "kubio_alt_svc_advertisements_total",
        &[("outcome", "skipped"), ("reason", "advertise_disabled")],
        snapshot.overview.alt_svc.skipped_advertise_disabled,
    );
    metric(
        &mut out,
        "kubio_alt_svc_advertisements_total",
        &[("outcome", "skipped"), ("reason", "missing_authority")],
        snapshot.overview.alt_svc.skipped_missing_authority,
    );
    metric(
        &mut out,
        "kubio_alt_svc_advertisements_total",
        &[("outcome", "skipped"), ("reason", "authority_not_allowed")],
        snapshot.overview.alt_svc.skipped_authority_not_allowed,
    );
    metric(
        &mut out,
        "kubio_alt_svc_advertisements_total",
        &[("outcome", "skipped"), ("reason", "invalid_value")],
        snapshot.overview.alt_svc.skipped_invalid_value,
    );
    line(
        &mut out,
        "kubio_http3_connections_total",
        "Downstream HTTP/3 connection outcomes.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_http3_connections_total",
        &[("outcome", "accepted")],
        snapshot.overview.http3_server.connections_accepted,
    );
    metric(
        &mut out,
        "kubio_http3_connections_total",
        &[("outcome", "handshake_failed")],
        snapshot.overview.http3_server.handshake_failures,
    );
    line(
        &mut out,
        "kubio_http3_streams_total",
        "Downstream HTTP/3 request stream outcomes.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_http3_streams_total",
        &[("outcome", "accepted")],
        snapshot.overview.http3_server.streams_accepted,
    );
    metric(
        &mut out,
        "kubio_http3_streams_total",
        &[("outcome", "malformed_request")],
        snapshot.overview.http3_server.malformed_requests,
    );
    metric(
        &mut out,
        "kubio_http3_streams_total",
        &[("outcome", "request_body_rejected")],
        snapshot.overview.http3_server.request_body_rejections,
    );
    line(
        &mut out,
        "kubio_http3_response_write_errors_total",
        "Downstream HTTP/3 response write errors by bounded phase.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_http3_response_write_errors_total",
        &[("phase", "headers")],
        snapshot.overview.http3_server.response_write_header_errors,
    );
    metric(
        &mut out,
        "kubio_http3_response_write_errors_total",
        &[("phase", "body")],
        snapshot.overview.http3_server.response_write_body_errors,
    );
    metric(
        &mut out,
        "kubio_http3_response_write_errors_total",
        &[("phase", "finish")],
        snapshot.overview.http3_server.response_finish_errors,
    );
    line(
        &mut out,
        "kubio_upstream_http3_requests_total",
        "Upstream HTTP/3 attempt and fallback outcomes.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_upstream_http3_requests_total",
        &[("outcome", "attempt")],
        snapshot.overview.upstream_http3.attempts,
    );
    metric(
        &mut out,
        "kubio_upstream_http3_requests_total",
        &[("outcome", "success")],
        snapshot.overview.upstream_http3.successes,
    );
    metric(
        &mut out,
        "kubio_upstream_http3_requests_total",
        &[("outcome", "failure")],
        snapshot.overview.upstream_http3.failures,
    );
    metric(
        &mut out,
        "kubio_upstream_http3_requests_total",
        &[("outcome", "fallback")],
        snapshot.overview.upstream_http3.fallbacks,
    );
    metric(
        &mut out,
        "kubio_upstream_http3_requests_total",
        &[("outcome", "required_failure")],
        snapshot.overview.upstream_http3.required_failures,
    );
    metric(
        &mut out,
        "kubio_upstream_http3_requests_total",
        &[("outcome", "skipped_not_https")],
        snapshot.overview.upstream_http3.skipped_not_https,
    );
    metric(
        &mut out,
        "kubio_upstream_http3_requests_total",
        &[("outcome", "skipped_non_replayable")],
        snapshot.overview.upstream_http3.skipped_non_replayable,
    );

    line(
        &mut out,
        "kubio_policy_decisions_total",
        "Policy decisions by route and decision.",
        "counter",
    );
    for route in &snapshot.routes {
        let route_id = sanitize_label(&route.route_id.as_label());
        if route.reuse_count > 0 {
            metric(
                &mut out,
                "kubio_policy_decisions_total",
                &[
                    ("route_id", &route_id),
                    ("decision", Decision::Reuse.to_string().as_str()),
                ],
                route.reuse_count,
            );
        }
        if route.protected_count > 0 {
            metric(
                &mut out,
                "kubio_policy_decisions_total",
                &[
                    ("route_id", &route_id),
                    ("decision", Decision::Protect.to_string().as_str()),
                ],
                route.protected_count,
            );
        }
        if route.bypass_count > 0 {
            metric(
                &mut out,
                "kubio_policy_decisions_total",
                &[
                    ("route_id", &route_id),
                    ("decision", Decision::Bypass.to_string().as_str()),
                ],
                route.bypass_count,
            );
        }
    }

    line(
        &mut out,
        "kubio_request_duration_seconds",
        "Request latency histogram from in-memory observations.",
        "histogram",
    );
    for route in &snapshot.routes {
        let route_id = sanitize_label(&route.route_id.as_label());
        histogram(
            &mut out,
            "kubio_request_duration_seconds",
            &route_id,
            &route.latency,
        );
    }

    line(
        &mut out,
        "kubio_origin_duration_seconds",
        "Origin latency histogram. v0.1.0 mirrors request duration.",
        "histogram",
    );
    for route in &snapshot.routes {
        let route_id = sanitize_label(&route.route_id.as_label());
        histogram(
            &mut out,
            "kubio_origin_duration_seconds",
            &route_id,
            &route.latency,
        );
    }

    line(
        &mut out,
        "kubio_status_class_total",
        "Observed responses by status class.",
        "counter",
    );
    for route in &snapshot.routes {
        let route_id = sanitize_label(&route.route_id.as_label());
        let counts = [
            (
                StatusClass::Informational,
                route.status_classes.informational,
            ),
            (StatusClass::Success, route.status_classes.success),
            (StatusClass::Redirection, route.status_classes.redirection),
            (StatusClass::ClientError, route.status_classes.client_error),
            (StatusClass::ServerError, route.status_classes.server_error),
            (StatusClass::Unknown, route.status_classes.unknown),
        ];
        for (class, count) in counts {
            if count > 0 {
                metric(
                    &mut out,
                    "kubio_status_class_total",
                    &[("route_id", &route_id), ("status_class", class.label())],
                    count,
                );
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kubio_core::{
        ConfidenceTier, LatencyBucketSnapshot, LatencySnapshot, ReuseClass, RouteId, RouteState,
        StatusClassCounts,
    };
    use kubio_observe::{ObserverSnapshot, OverviewSnapshot, ProtocolCounts, RouteSnapshot};
    use kubio_store::{StoreKind, StoreOperationMetrics, StoreStats};

    #[test]
    fn sanitizer_removes_metric_breakouts() {
        assert_eq!(sanitize_label("a\nb\"c\\d"), "a_b_c_d");
    }

    #[test]
    fn latency_metrics_render_as_histograms() {
        let snapshot = ObserverSnapshot {
            overview: OverviewSnapshot::default(),
            routes: vec![RouteSnapshot {
                route_id: RouteId::new("GET", "/api/products"),
                route_hash: "hash".to_string(),
                state: RouteState::Watching,
                reuse_class: ReuseClass::Watching,
                request_count: 2,
                origin_count: 2,
                reuse_count: 0,
                protected_count: 0,
                bypass_count: 0,
                store_safe_count: 0,
                origin_public_responses: 0,
                distinct_key_count: 0,
                dynamic_value_count: 0,
                slug_value_count: 0,
                store_safe_rate: 0.0,
                adaptive_blockers: vec![],
                confidence_tier: ConfidenceTier::Unknown,
                evidence_window_age_seconds: 0,
                stale_evidence: false,
                cooldown_remaining_seconds: None,
                canary_matches: 0,
                canary_mismatches: 0,
                query_equivalence_candidates: 0,
                query_compacted_groups: 0,
                ignored_response_header_count: 0,
                suppressed_on_hit_header_count: 0,
                verified_header_ignore_candidates: 0,
                variant_dimensions: 0,
                variant_unbounded: false,
                shadow_matches: 0,
                shadow_mismatches: 0,
                revalidation_attempts: 0,
                revalidation_not_modified: 0,
                revalidation_modified: 0,
                revalidation_failed: 0,
                stale_served: 0,
                stale_denied: 0,
                route_hint_applied: 0,
                route_hint_rejected: 0,
                query_hint_applied: 0,
                query_hint_rejected: 0,
                query_param_suggestions: 0,
                downstream_protocols: ProtocolCounts::default(),
                upstream_protocols: ProtocolCounts::default(),
                status_classes: StatusClassCounts::default(),
                latency: LatencySnapshot {
                    p50_ms: 1.0,
                    p95_ms: 2.0,
                    avg_ms: 1.5,
                    count: 2,
                    sum_ms: 3.0,
                    buckets: vec![LatencyBucketSnapshot {
                        le_seconds: 0.005,
                        count: 2,
                    }],
                },
                repeat_rate: 0.0,
                estimated_savings: 0.0,
                actual_reuse_rate: 0.0,
                score: 0,
                reasons: vec![],
                explanation: vec![],
                route_hint: None,
                query_params: vec![],
                response_headers: vec![],
            }],
            events: vec![],
        };
        let metrics = render_metrics(
            &snapshot,
            &StoreStats {
                entries: 0,
                bytes: 0,
                evictions: 0,
                max_size: 1,
                max_object_size: 1,
                kind: StoreKind::Memory,
                disk_path: None,
                startup_recovered_entries: None,
                corrupt_entries_skipped: None,
                operations: StoreOperationMetrics::default(),
            },
        );

        assert!(metrics.contains("kubio_request_duration_seconds_bucket"));
        assert!(metrics.contains("kubio_request_duration_seconds_sum"));
        assert!(metrics.contains("kubio_request_duration_seconds_count"));
        assert!(metrics.contains("kubio_route_hints_applied_total"));
        assert!(metrics.contains("kubio_query_hints_applied_total"));
        assert!(metrics.contains("kubio_routes_by_reuse_class"));
        assert!(metrics.contains("kubio_origin_public_fast_path_total"));
        assert!(metrics.contains("kubio_precision_confidence_routes"));
        assert!(metrics.contains("kubio_precision_canary_total"));
        assert!(metrics.contains("kubio_query_equivalence_candidates_total"));
        assert!(metrics.contains("kubio_response_header_equivalence_candidates_total"));
        assert!(metrics.contains("kubio_response_header_ignored_total"));
        assert!(metrics.contains("kubio_response_header_suppressed_on_hit_total"));
        assert!(metrics.contains("kubio_variant_groups"));
        assert!(metrics.contains("kubio_store_errors_total"));
        assert!(metrics.contains("kubio_store_operations_total"));
        assert!(metrics.contains("kubio_store_operation_duration_seconds_sum"));
        assert!(metrics.contains("kubio_store_saturation_events_total"));
        assert!(metrics.contains("kubio_downstream_requests_total"));
        assert!(metrics.contains("kubio_upstream_requests_total"));
        assert!(metrics.contains("kubio_backpressure_rejections_total"));
        assert!(metrics.contains("kubio_in_flight_requests"));
        assert!(metrics.contains("kubio_protocol_fallbacks_total"));
        assert!(metrics.contains("kubio_observer_events_dropped_total"));
        assert!(metrics.contains("kubio_alt_svc_advertisements_total"));
        assert!(metrics.contains("kubio_http3_connections_total"));
        assert!(metrics.contains("kubio_http3_response_write_errors_total"));
        assert!(metrics.contains("kubio_upstream_http3_requests_total"));
    }
}
