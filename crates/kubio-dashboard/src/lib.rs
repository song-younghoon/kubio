//! Local dashboard and admin API.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kubio_core::{CacheKeyHash, DecisionReason, EffectiveConfig, RedactedConfig, RouteId};
use kubio_observe::{EventType, Observer, ObserverSnapshot};
use kubio_store::{CacheStore, PurgeResult, PurgeSelector, StoreStats};
use kubio_telemetry::render_metrics;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;
use tokio::net::TcpListener;

#[derive(Clone)]
pub struct DashboardState {
    pub config: Arc<EffectiveConfig>,
    pub observer: Arc<Observer>,
    pub store: Arc<dyn CacheStore>,
}

pub fn router(state: DashboardState) -> Router {
    let mut router = Router::new()
        .route("/", get(index))
        .route("/routes", get(routes_page))
        .route("/routes/{route_hash}", get(route_page))
        .route("/events", get(events_page))
        .route("/config", get(config_page))
        .route("/api/overview", get(api_overview))
        .route("/api/routes", get(api_routes))
        .route("/api/routes/by-hash/{route_hash}", get(api_route_detail))
        .route("/api/events", get(api_events))
        .route("/api/config", get(api_config))
        .route("/api/purge", post(api_purge));

    if state.config.observability.metrics {
        router = router.route(&state.config.observability.metrics_path, get(metrics));
    }

    router.fallback(not_found).with_state(state)
}

pub async fn run_dashboard<F>(state: DashboardState, shutdown: F) -> anyhow::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind(state.config.dashboard.listen).await?;
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

async fn index(State(state): State<DashboardState>) -> Html<String> {
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
        ),
    ))
}

async fn routes_page(State(state): State<DashboardState>) -> Html<String> {
    let snapshot = state.observer.snapshot();
    let rows = snapshot
        .routes
        .iter()
        .map(|route| {
            format!(
                "<tr><td><a href=\"/routes/{hash}\">{label}</a></td><td>{state}</td><td>{requests}</td><td>{origin}</td><td>{reuse}</td><td>{protected}</td></tr>",
                hash = route.route_hash,
                label = escape_html(&route.route_id.as_label()),
                state = route.state,
                requests = route.request_count,
                origin = route.origin_count,
                reuse = route.reuse_count,
                protected = route.protected_count,
            )
        })
        .collect::<String>();
    Html(layout(
        "Routes",
        &format!(
            "<table><thead><tr><th>Route</th><th>Status</th><th>Requests</th><th>Origin</th><th>Reused</th><th>Protected</th></tr></thead><tbody>{rows}</tbody></table>"
        ),
    ))
}

async fn route_page(
    State(state): State<DashboardState>,
    Path(route_hash): Path<String>,
) -> Response {
    let Some(route) = state.observer.route_by_hash(&route_hash) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let reasons = route
        .explanation
        .iter()
        .map(|reason| format!("<li>{}</li>", escape_html(reason)))
        .collect::<String>();
    Html(layout(
        &route.route_id.as_label(),
        &format!(
            r#"
<section>
  <h2>{}</h2>
  <p>Status: {}</p>
  <h3>kubio's reasoning</h3>
  <ul>{}</ul>
  <dl>
    <dt>Requests</dt><dd>{}</dd>
    <dt>Origin requests</dt><dd>{}</dd>
    <dt>Reused responses</dt><dd>{}</dd>
    <dt>Shadow matches</dt><dd>{}</dd>
    <dt>Shadow mismatches</dt><dd>{}</dd>
    <dt>p95 latency</dt><dd>{:.2} ms</dd>
  </dl>
</section>
"#,
            escape_html(&route.route_id.as_label()),
            route.state,
            reasons,
            route.request_count,
            route.origin_count,
            route.reuse_count,
            route.shadow_matches,
            route.shadow_mismatches,
            route.latency.p95_ms,
        ),
    ))
    .into_response()
}

async fn events_page(State(state): State<DashboardState>) -> Html<String> {
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

async fn config_page(State(state): State<DashboardState>) -> Html<String> {
    let body = serde_json::to_string_pretty(&state.config.redacted()).unwrap_or_default();
    Html(layout(
        "Config",
        &format!("<pre>{}</pre>", escape_html(&body)),
    ))
}

async fn api_overview(State(state): State<DashboardState>) -> Json<OverviewResponse> {
    let snapshot = state.observer.snapshot();
    Json(OverviewResponse {
        mode: state.config.mode.to_string(),
        origin: state.config.origin.to_string(),
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
        p50_latency_ms: snapshot.overview.p50_latency_ms,
        p95_latency_ms: snapshot.overview.p95_latency_ms,
        cache_entries: state.store.stats().entries,
        cache_bytes: state.store.stats().bytes,
    })
}

async fn api_routes(State(state): State<DashboardState>) -> Json<ObserverSnapshot> {
    Json(state.observer.snapshot())
}

async fn api_route_detail(
    State(state): State<DashboardState>,
    Path(route_hash): Path<String>,
) -> Response {
    match state.observer.route_by_hash(&route_hash) {
        Some(route) => Json(route).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn api_events(State(state): State<DashboardState>) -> Json<ObserverSnapshot> {
    Json(state.observer.snapshot())
}

async fn api_config(State(state): State<DashboardState>) -> Json<RedactedConfig> {
    Json(state.config.redacted())
}

async fn api_purge(
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

async fn metrics(State(state): State<DashboardState>) -> Response {
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

async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "not found")
}

fn authorized(config: &EffectiveConfig, headers: &HeaderMap) -> bool {
    if !admin_token_required(config) {
        return true;
    }
    let Some(expected) = config.admin_token.as_deref() else {
        return false;
    };
    headers
        .get("x-kubio-admin-token")
        .and_then(|value| value.to_str().ok())
        .map(|actual| actual == expected)
        .unwrap_or(false)
}

fn admin_token_required(config: &EffectiveConfig) -> bool {
    config.dashboard.allow_public || !config.dashboard.listen.ip().is_loopback()
}

fn parse_route_id(value: &str) -> Option<RouteId> {
    let (method, path) = value.split_once(' ')?;
    Some(RouteId::new(method, path))
}

fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>kubio - {}</title>
<style>
body {{ font-family: system-ui, sans-serif; margin: 2rem; color: #17202a; }}
nav a {{ margin-right: 1rem; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ border-bottom: 1px solid #d7dee8; padding: .5rem; text-align: left; }}
dt {{ font-weight: 700; }}
dd {{ margin: 0 0 .75rem 0; }}
pre {{ background: #f6f8fa; padding: 1rem; overflow: auto; }}
</style>
</head>
<body>
<nav><a href="/">Overview</a><a href="/routes">Routes</a><a href="/events">Events</a><a href="/config">Config</a></nav>
<main>{}</main>
</body>
</html>"#,
        escape_html(title),
        body
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

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
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub cache_entries: u64,
    pub cache_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PurgeRequest {
    pub selector: String,
    pub route_id: Option<String>,
    pub cache_key_hash: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
struct StoreView {
    stats: StoreStats,
    last_purge: Option<PurgeResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use kubio_core::{EffectiveConfig, StorageConfig};
    use kubio_store::MemoryStore;
    use tower::ServiceExt;

    #[test]
    fn route_id_parser_accepts_method_space_path() {
        let route = parse_route_id("GET /api/products").unwrap();
        assert_eq!(route.method, "GET");
        assert_eq!(route.template, "/api/products");
    }

    #[test]
    fn html_escape_handles_sensitive_chars() {
        assert_eq!(escape_html("<x&y>"), "&lt;x&amp;y&gt;");
    }

    #[tokio::test]
    async fn metrics_path_can_be_configured() {
        let mut config = EffectiveConfig::default();
        config.observability.metrics_path = "/internal/metrics".to_string();
        let app = router(test_state(config));

        let configured = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/internal/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let default = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(configured.status(), StatusCode::OK);
        assert_eq!(default.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn metrics_can_be_disabled() {
        let mut config = EffectiveConfig::default();
        config.observability.metrics = false;
        let app = router(test_state(config));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    fn test_state(config: EffectiveConfig) -> DashboardState {
        let config = Arc::new(config);
        DashboardState {
            config: config.clone(),
            observer: Arc::new(Observer::new(100, 100, 100, 2, 2, 1)),
            store: Arc::new(MemoryStore::new(&StorageConfig::default())),
        }
    }
}
