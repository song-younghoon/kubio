use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use kubio_core::{
    ActiveConfigResponse, CacheKeyHash, ConfigCheckRequest, ConfigReloadRequest,
    ConfigReloadSnapshot, DecisionReason, RedactedConfig,
};
use kubio_observe::{EventType, ObserverSnapshot};
use kubio_store::{PurgeSelector, StoreStats};
use kubio_telemetry::render_metrics;

use crate::auth::authorized;
use crate::models::{parse_route_id, OverviewResponse, PurgeRequest};
use crate::state::DashboardState;

pub(crate) async fn api_overview(State(state): State<DashboardState>) -> Json<OverviewResponse> {
    let snapshot = state.observer.snapshot();
    let active = state.active_config();
    let reload = state.reload_status();
    Json(OverviewResponse {
        mode: active.config.mode.to_string(),
        origin: active.config.origin.to_string(),
        observed_requests: snapshot.overview.observed_requests,
        origin_requests: snapshot.overview.origin_requests,
        reused_responses: snapshot.overview.reused_responses,
        protected_requests: snapshot.overview.protected_requests,
        bypassed_requests: snapshot.overview.bypassed_requests,
        candidate_routes: snapshot.overview.candidate_routes,
        auto_routes: snapshot.overview.auto_routes,
        estimated_savings: snapshot.overview.estimated_savings,
        actual_reuse_rate: snapshot.overview.actual_reuse_rate,
        shadow_matches: snapshot.overview.shadow_matches,
        shadow_mismatches: snapshot.overview.shadow_mismatches,
        revalidation_attempts: snapshot.overview.revalidation_attempts,
        revalidation_not_modified: snapshot.overview.revalidation_not_modified,
        revalidation_modified: snapshot.overview.revalidation_modified,
        revalidation_failed: snapshot.overview.revalidation_failed,
        stale_responses_served: snapshot.overview.stale_responses_served,
        stale_responses_denied: snapshot.overview.stale_responses_denied,
        route_hints_applied: snapshot.overview.route_hints_applied,
        route_hints_rejected: snapshot.overview.route_hints_rejected,
        query_hints_applied: snapshot.overview.query_hints_applied,
        query_hints_rejected: snapshot.overview.query_hints_rejected,
        query_param_suggestions: snapshot.overview.query_param_suggestions,
        store_errors: snapshot.overview.store_errors,
        dropped_events: snapshot.overview.dropped_events,
        backpressure_rejections: snapshot.overview.backpressure_rejections,
        protocol_fallbacks: snapshot.overview.protocol_fallbacks,
        in_flight_requests: snapshot.overview.in_flight_requests,
        max_in_flight_requests: snapshot.overview.max_in_flight_requests,
        downstream_http1_requests: snapshot.overview.downstream_http1_requests,
        downstream_http2_requests: snapshot.overview.downstream_http2_requests,
        downstream_http3_requests: snapshot.overview.downstream_http3_requests,
        upstream_http1_requests: snapshot.overview.upstream_http1_requests,
        upstream_http2_requests: snapshot.overview.upstream_http2_requests,
        upstream_http3_requests: snapshot.overview.upstream_http3_requests,
        alt_svc: snapshot.overview.alt_svc,
        http3_server: snapshot.overview.http3_server,
        upstream_http3: snapshot.overview.upstream_http3,
        p50_latency_ms: snapshot.overview.p50_latency_ms,
        p95_latency_ms: snapshot.overview.p95_latency_ms,
        cache_entries: state.store.stats().entries,
        cache_bytes: state.store.stats().bytes,
        store_kind: format!("{:?}", state.store.stats().kind).to_ascii_lowercase(),
        config_generation: active.generation,
        last_reload_status: reload.last_status.map(|status| status.to_string()),
        last_reloadable_changes: reload.last_reloadable_change_count,
        last_restart_required: reload.last_restart_required_count,
        last_routes_demoted: reload.last_routes_demoted,
        last_cache_entries_purged: reload.last_cache_entries_purged,
    })
}

pub(crate) async fn api_routes(State(state): State<DashboardState>) -> Json<ObserverSnapshot> {
    Json(state.observer.snapshot())
}

pub(crate) async fn api_route_detail(
    State(state): State<DashboardState>,
    Path(route_hash): Path<String>,
) -> Response {
    match state.observer.route_by_hash(&route_hash) {
        Some(route) => Json(route).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) async fn api_events(State(state): State<DashboardState>) -> Json<ObserverSnapshot> {
    Json(state.observer.snapshot())
}

pub(crate) async fn api_config(State(state): State<DashboardState>) -> Json<RedactedConfig> {
    Json(state.active_config().config)
}

pub(crate) async fn api_active_config(
    State(state): State<DashboardState>,
) -> Json<ActiveConfigResponse> {
    Json(state.active_config())
}

pub(crate) async fn api_reload_status(
    State(state): State<DashboardState>,
) -> Json<ConfigReloadSnapshot> {
    Json(state.reload_status())
}

pub(crate) async fn api_reload_config(
    State(state): State<DashboardState>,
    headers: HeaderMap,
    Json(request): Json<ConfigReloadRequest>,
) -> Response {
    if !state.config.dashboard.admin_api {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !authorized(&state.config, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.reloader.as_ref() {
        Some(reloader) => Json(reloader.reload_config(request).await).into_response(),
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

pub(crate) async fn api_check_config(
    State(state): State<DashboardState>,
    headers: HeaderMap,
    Json(request): Json<ConfigCheckRequest>,
) -> Response {
    if !state.config.dashboard.admin_api {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !authorized(&state.config, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.reloader.as_ref() {
        Some(reloader) => Json(reloader.check_config(request).await).into_response(),
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

pub(crate) async fn api_store(State(state): State<DashboardState>) -> Json<StoreStats> {
    Json(state.store.stats())
}

pub(crate) async fn api_purge(
    State(state): State<DashboardState>,
    headers: HeaderMap,
    Json(request): Json<PurgeRequest>,
) -> Response {
    if !state.config.dashboard.admin_api {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !authorized(&state.config, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let selector = match request.selector.as_str() {
        "all" => PurgeSelector::All,
        "route" => {
            let Some(route_id) = request.route_id.as_deref().and_then(parse_route_id) else {
                return (StatusCode::BAD_REQUEST, "route_id is required").into_response();
            };
            PurgeSelector::Route(route_id)
        }
        "key" => {
            let Some(key) = request.cache_key_hash else {
                return (StatusCode::BAD_REQUEST, "cache_key_hash is required").into_response();
            };
            PurgeSelector::Key(CacheKeyHash(key))
        }
        _ => return (StatusCode::BAD_REQUEST, "unsupported purge selector").into_response(),
    };

    match state.store.purge(selector).await {
        Ok(result) => {
            state.observer.push_event(
                EventType::CacheEntryEvicted,
                None,
                None,
                vec![DecisionReason::StoreError],
                format!("purged {} cache entries", result.purged_entries),
            );
            Json(result).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("purge failed: {err}"),
        )
            .into_response(),
    }
}

pub(crate) async fn metrics(State(state): State<DashboardState>) -> Response {
    let metrics = render_metrics(&state.observer.snapshot(), &state.store.stats());
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        metrics,
    )
        .into_response()
}
