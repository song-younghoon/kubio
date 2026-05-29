//! Logging and Prometheus text rendering helpers.

use kubio_core::{Decision, StatusClass};
use kubio_observe::ObserverSnapshot;
use kubio_store::StoreStats;
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
    metric(&mut out, "kubio_cache_entries", &[], store.entries);
    line(&mut out, "kubio_cache_bytes", "Cache bytes.", "gauge");
    metric(&mut out, "kubio_cache_bytes", &[], store.bytes);
    line(
        &mut out,
        "kubio_cache_evictions_total",
        "Cache evictions.",
        "counter",
    );
    metric(
        &mut out,
        "kubio_cache_evictions_total",
        &[],
        store.evictions,
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
        "Approximate request latency quantiles from in-memory observations.",
        "gauge",
    );
    for route in &snapshot.routes {
        let route_id = sanitize_label(&route.route_id.as_label());
        metric_f64(
            &mut out,
            "kubio_request_duration_seconds",
            &[("route_id", &route_id), ("quantile", "0.50")],
            route.latency.p50_ms / 1000.0,
        );
        metric_f64(
            &mut out,
            "kubio_request_duration_seconds",
            &[("route_id", &route_id), ("quantile", "0.95")],
            route.latency.p95_ms / 1000.0,
        );
    }

    line(
        &mut out,
        "kubio_origin_duration_seconds",
        "Approximate origin latency quantiles. v0.1.0 mirrors request duration.",
        "gauge",
    );
    for route in &snapshot.routes {
        let route_id = sanitize_label(&route.route_id.as_label());
        metric_f64(
            &mut out,
            "kubio_origin_duration_seconds",
            &[("route_id", &route_id), ("quantile", "0.95")],
            route.latency.p95_ms / 1000.0,
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

fn metric_f64(out: &mut String, name: &str, labels: &[(&str, &str)], value: f64) {
    out.push_str(name);
    push_labels(out, labels);
    out.push(' ');
    out.push_str(&format!("{value:.6}"));
    out.push('\n');
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

    #[test]
    fn sanitizer_removes_metric_breakouts() {
        assert_eq!(sanitize_label("a\nb\"c\\d"), "a_b_c_d");
    }
}
