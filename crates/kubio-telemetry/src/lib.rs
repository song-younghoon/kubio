//! Logging and Prometheus text rendering helpers.

use kubio_core::{Decision, StatusClass};
use kubio_observe::ObserverSnapshot;
use kubio_store::{StoreKind, StoreStats};
use tracing_subscriber::EnvFilter;

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("kubio=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

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

fn store_kind_label(kind: StoreKind) -> &'static str {
    match kind {
        StoreKind::Memory => "memory",
        StoreKind::Disk => "disk",
    }
}

pub fn sanitize_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '"' | '\\' | '\n' | '\r' | '\t' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect()
}

fn line(out: &mut String, name: &str, help: &str, kind: &str) {
    out.push_str("# HELP ");
    out.push_str(name);
    out.push(' ');
    out.push_str(help);
    out.push('\n');
    out.push_str("# TYPE ");
    out.push_str(name);
    out.push(' ');
    out.push_str(kind);
    out.push('\n');
}

fn metric(out: &mut String, name: &str, labels: &[(&str, &str)], value: u64) {
    out.push_str(name);
    push_labels(out, labels);
    out.push(' ');
    out.push_str(&value.to_string());
    out.push('\n');
}

fn histogram(out: &mut String, name: &str, route_id: &str, latency: &kubio_core::LatencySnapshot) {
    let bucket_name = format!("{name}_bucket");
    for bucket in &latency.buckets {
        metric(
            out,
            &bucket_name,
            &[
                ("route_id", route_id),
                ("le", &format!("{:.3}", bucket.le_seconds)),
            ],
            bucket.count,
        );
    }
    metric(
        out,
        &bucket_name,
        &[("route_id", route_id), ("le", "+Inf")],
        latency.count,
    );
    out.push_str(name);
    out.push_str("_sum");
    push_labels(out, &[("route_id", route_id)]);
    out.push(' ');
    out.push_str(&format!("{:.6}", latency.sum_ms / 1000.0));
    out.push('\n');
    metric(
        out,
        &format!("{name}_count"),
        &[("route_id", route_id)],
        latency.count,
    );
}

fn push_labels(out: &mut String, labels: &[(&str, &str)]) {
    if labels.is_empty() {
        return;
    }
    out.push('{');
    for (index, (name, value)) in labels.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(&sanitize_label(value));
        out.push('"');
    }
    out.push('}');
}

#[cfg(test)]
mod tests {
    use super::*;
    use kubio_core::{
        LatencyBucketSnapshot, LatencySnapshot, RouteId, RouteState, StatusClassCounts,
    };
    use kubio_observe::{ObserverSnapshot, OverviewSnapshot, RouteSnapshot};
    use kubio_store::StoreStats;

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
                request_count: 2,
                origin_count: 2,
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
                route_hint_applied: 0,
                route_hint_rejected: 0,
                query_hint_applied: 0,
                query_hint_rejected: 0,
                query_param_suggestions: 0,
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
            },
        );

        assert!(metrics.contains("kubio_request_duration_seconds_bucket"));
        assert!(metrics.contains("kubio_request_duration_seconds_sum"));
        assert!(metrics.contains("kubio_request_duration_seconds_count"));
        assert!(metrics.contains("kubio_route_hints_applied_total"));
        assert!(metrics.contains("kubio_query_hints_applied_total"));
        assert!(metrics.contains("kubio_store_errors_total"));
    }
}
