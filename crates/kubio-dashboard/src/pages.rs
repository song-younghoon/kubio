use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::html::{escape_html, layout, protocol_counts_html};
use crate::state::DashboardState;

pub(crate) async fn index(State(state): State<DashboardState>) -> Html<String> {
    let snapshot = state.observer.snapshot();
    Html(layout(
        "Overview",
        &format!(
            r#"
<section>
  <h2>kubio is watching your API</h2>
  <dl>
    <dt>Mode</dt><dd>{}</dd>
    <dt>Origin</dt><dd>{}</dd>
    <dt>Observed requests</dt><dd>{}</dd>
    <dt>Origin requests</dt><dd>{}</dd>
    <dt>Reused responses</dt><dd>{}</dd>
    <dt>Protected requests</dt><dd>{}</dd>
    <dt>Candidate routes</dt><dd>{}</dd>
    <dt>Auto routes</dt><dd>{}</dd>
    <dt>Shadow matches</dt><dd>{}</dd>
    <dt>Shadow mismatches</dt><dd>{}</dd>
    <dt>Revalidated</dt><dd>{}</dd>
    <dt>Stale served</dt><dd>{}</dd>
    <dt>Backpressure rejections</dt><dd>{}</dd>
    <dt>Protocol fallbacks</dt><dd>{}</dd>
    <dt>In-flight requests</dt><dd>{}/{}</dd>
    <dt>Downstream protocols</dt><dd>h1 {} / h2 {} / h3 {}</dd>
    <dt>Upstream protocols</dt><dd>h1 {} / h2 {} / h3 {}</dd>
    <dt>HTTP/3 connections</dt><dd>accepted {} / handshake failed {}</dd>
    <dt>Alt-Svc</dt><dd>advertised {} / authority skipped {}</dd>
    <dt>Upstream HTTP/3</dt><dd>attempts {} / success {} / fallback {}</dd>
    <dt>Store</dt><dd>{:?}</dd>
  </dl>
</section>
"#,
            state.config.mode,
            state.config.origin,
            snapshot.overview.observed_requests,
            snapshot.overview.origin_requests,
            snapshot.overview.reused_responses,
            snapshot.overview.protected_requests,
            snapshot.overview.candidate_routes,
            snapshot.overview.auto_routes,
            snapshot.overview.shadow_matches,
            snapshot.overview.shadow_mismatches,
            snapshot.overview.revalidation_attempts,
            snapshot.overview.stale_responses_served,
            snapshot.overview.backpressure_rejections,
            snapshot.overview.protocol_fallbacks,
            snapshot.overview.in_flight_requests,
            snapshot.overview.max_in_flight_requests,
            snapshot.overview.downstream_http1_requests,
            snapshot.overview.downstream_http2_requests,
            snapshot.overview.downstream_http3_requests,
            snapshot.overview.upstream_http1_requests,
            snapshot.overview.upstream_http2_requests,
            snapshot.overview.upstream_http3_requests,
            snapshot.overview.http3_server.connections_accepted,
            snapshot.overview.http3_server.handshake_failures,
            snapshot.overview.alt_svc.advertised,
            snapshot.overview.alt_svc.skipped_authority_not_allowed,
            snapshot.overview.upstream_http3.attempts,
            snapshot.overview.upstream_http3.successes,
            snapshot.overview.upstream_http3.fallbacks,
            state.store.stats().kind,
        ),
    ))
}

pub(crate) async fn routes_page(State(state): State<DashboardState>) -> Html<String> {
    let snapshot = state.observer.snapshot();
    let rows = snapshot
        .routes
        .iter()
        .map(|route| {
            format!(
                "<tr><td><a href=\"/routes/{hash}\">{label}</a></td><td>{state}</td><td>{class}</td><td>{confidence}</td><td>{requests}</td><td>{origin}</td><td>{reuse}</td><td>{protected}</td><td>{keys}</td><td>{query}</td><td>{headers}</td><td>{downstream}</td><td>{upstream}</td></tr>",
                hash = route.route_hash,
                label = escape_html(&route.route_id.as_label()),
                state = route.state,
                class = route.reuse_class,
                confidence = route.confidence_tier,
                requests = route.request_count,
                origin = route.origin_count,
                reuse = route.reuse_count,
                protected = route.protected_count,
                keys = route.distinct_key_count,
                query = route.query_equivalence_candidates,
                headers = route.ignored_response_header_count,
                downstream = protocol_counts_html(&route.downstream_protocols),
                upstream = protocol_counts_html(&route.upstream_protocols),
            )
        })
        .collect::<String>();
    Html(layout(
        "Routes",
        &format!(
            "<table><thead><tr><th>Route</th><th>Status</th><th>Reuse class</th><th>Confidence</th><th>Requests</th><th>Origin</th><th>Reused</th><th>Protected</th><th>Keys</th><th>Query candidates</th><th>Header ignored</th><th>Downstream</th><th>Upstream</th></tr></thead><tbody>{rows}</tbody></table>"
        ),
    ))
}

pub(crate) async fn route_page(
    State(state): State<DashboardState>,
    Path(route_hash): Path<String>,
) -> Response {
    let Some(route) = state.observer.route_by_hash(&route_hash) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let snapshot = state.observer.snapshot();
    let reasons = route
        .explanation
        .iter()
        .map(|reason| format!("<li>{}</li>", escape_html(reason)))
        .collect::<String>();
    let events = snapshot
        .events
        .iter()
        .rev()
        .filter(|event| event.route_id.as_ref() == Some(&route.route_id))
        .take(5)
        .map(|event| {
            format!(
                "<li>{:?}: {}</li>",
                event.event_type,
                escape_html(&event.message)
            )
        })
        .collect::<String>();
    let blockers = route
        .adaptive_blockers
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let query_params = route
        .query_params
        .iter()
        .map(|param| {
            format!(
                "<li>{}: {} values={} matches={} mismatches={} enabled={}</li>",
                escape_html(&param.name),
                param.equivalence_class,
                param.distinct_value_count,
                param.matching_fingerprint_count,
                param.mismatch_count,
                param.operator_enabled,
            )
        })
        .collect::<String>();
    let response_headers = route
        .response_headers
        .iter()
        .map(|header| {
            format!(
                "<li>{}: {} values={} matches={} mismatches={} suppressed_on_hit={}</li>",
                escape_html(&header.name),
                header.class,
                header.distinct_value_count,
                header.matching_without_header_count,
                header.mismatch_count,
                header.suppressed_on_hit,
            )
        })
        .collect::<String>();
    Html(layout(
        &route.route_id.as_label(),
        &format!(
            r#"
<section>
  <h2>{}</h2>
  <p>Status: {}</p>
  <p>Reuse class: {}</p>
  <p>Confidence: {}</p>
  <h3>kubio's reasoning</h3>
  <ul>{}</ul>
  <dl>
    <dt>Requests</dt><dd>{}</dd>
    <dt>Origin requests</dt><dd>{}</dd>
    <dt>Reused responses</dt><dd>{}</dd>
    <dt>Distinct keys</dt><dd>{}</dd>
    <dt>Dynamic path values</dt><dd>{}</dd>
    <dt>Slug path values</dt><dd>{}</dd>
    <dt>Store-safe rate</dt><dd>{:.2}%</dd>
    <dt>Origin public responses</dt><dd>{}</dd>
    <dt>Evidence age</dt><dd>{}s</dd>
    <dt>Cooldown remaining</dt><dd>{}</dd>
    <dt>Canary</dt><dd>match {} / mismatch {}</dd>
    <dt>Query equivalence candidates</dt><dd>{}</dd>
    <dt>Query compacted groups</dt><dd>{}</dd>
    <dt>Response headers ignored</dt><dd>{}</dd>
    <dt>Response header candidates</dt><dd>{}</dd>
    <dt>Response headers suppressed on hit</dt><dd>{}</dd>
    <dt>Variant dimensions</dt><dd>{}</dd>
    <dt>Variant unbounded</dt><dd>{}</dd>
    <dt>Adaptive blockers</dt><dd>{}</dd>
    <dt>Shadow matches</dt><dd>{}</dd>
    <dt>Shadow mismatches</dt><dd>{}</dd>
    <dt>Revalidated</dt><dd>{}</dd>
    <dt>Stale served</dt><dd>{}</dd>
    <dt>Downstream protocols</dt><dd>{}</dd>
    <dt>Upstream protocols</dt><dd>{}</dd>
    <dt>p95 latency</dt><dd>{:.2} ms</dd>
  </dl>
  <h3>Query equivalence</h3>
  <ul>{}</ul>
  <h3>Response header equivalence</h3>
  <ul>{}</ul>
  <h3>Recent bounded events</h3>
  <ul>{}</ul>
</section>
"#,
            escape_html(&route.route_id.as_label()),
            route.state,
            route.reuse_class,
            route.confidence_tier,
            reasons,
            route.request_count,
            route.origin_count,
            route.reuse_count,
            route.distinct_key_count,
            route.dynamic_value_count,
            route.slug_value_count,
            route.store_safe_rate * 100.0,
            route.origin_public_responses,
            route.evidence_window_age_seconds,
            escape_html(
                &route
                    .cooldown_remaining_seconds
                    .map(|value| format!("{value}s"))
                    .unwrap_or_else(|| "none".to_string())
            ),
            route.canary_matches,
            route.canary_mismatches,
            route.query_equivalence_candidates,
            route.query_compacted_groups,
            route.ignored_response_header_count,
            route.verified_header_ignore_candidates,
            route.suppressed_on_hit_header_count,
            route.variant_dimensions,
            route.variant_unbounded,
            escape_html(if blockers.is_empty() {
                "none"
            } else {
                &blockers
            }),
            route.shadow_matches,
            route.shadow_mismatches,
            route.revalidation_attempts,
            route.stale_served,
            protocol_counts_html(&route.downstream_protocols),
            protocol_counts_html(&route.upstream_protocols),
            route.latency.p95_ms,
            query_params,
            response_headers,
            events,
        ),
    ))
    .into_response()
}

pub(crate) async fn events_page(State(state): State<DashboardState>) -> Html<String> {
    let snapshot = state.observer.snapshot();
    let rows = snapshot
        .events
        .iter()
        .rev()
        .map(|event| {
            format!(
                "<tr><td>{:?}</td><td>{:?}</td><td>{}</td></tr>",
                event.timestamp,
                event.event_type,
                escape_html(&event.message)
            )
        })
        .collect::<String>();
    Html(layout(
        "Events",
        &format!(
            "<table><thead><tr><th>Time</th><th>Event</th><th>Message</th></tr></thead><tbody>{rows}</tbody></table>"
        ),
    ))
}

pub(crate) async fn config_page(State(state): State<DashboardState>) -> Html<String> {
    let body = serde_json::to_string_pretty(&state.config.redacted()).unwrap_or_default();
    Html(layout(
        "Config",
        &format!("<pre>{}</pre>", escape_html(&body)),
    ))
}

pub(crate) async fn store_page(State(state): State<DashboardState>) -> Html<String> {
    let body = serde_json::to_string_pretty(&state.store.stats()).unwrap_or_default();
    Html(layout(
        "Store",
        &format!("<pre>{}</pre>", escape_html(&body)),
    ))
}
