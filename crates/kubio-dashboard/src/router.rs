use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use std::future::Future;
use tokio::net::TcpListener;

use crate::api::{
    api_active_config, api_check_config, api_config, api_events, api_overview, api_purge,
    api_reload_config, api_reload_status, api_route_detail, api_routes, api_store, metrics,
};
use crate::pages::{config_page, events_page, index, route_page, routes_page, store_page};
use crate::state::DashboardState;

pub fn router(state: DashboardState) -> Router {
    let mut router = Router::new()
        .route("/", get(index))
        .route("/routes", get(routes_page))
        .route("/routes/{route_hash}", get(route_page))
        .route("/events", get(events_page))
        .route("/config", get(config_page))
        .route("/store", get(store_page))
        .route("/api/overview", get(api_overview))
        .route("/api/routes", get(api_routes))
        .route("/api/routes/by-hash/{route_hash}", get(api_route_detail))
        .route("/api/events", get(api_events))
        .route("/api/config", get(api_config))
        .route("/api/config/active", get(api_active_config))
        .route("/api/config/reload-status", get(api_reload_status))
        .route("/api/config/reload", post(api_reload_config))
        .route("/api/config/check", post(api_check_config))
        .route("/api/store", get(api_store))
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

async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use kubio_core::{EffectiveConfig, StorageConfig};
    use kubio_observe::Observer;
    use kubio_store::MemoryStore;
    use std::sync::Arc;
    use tower::ServiceExt;

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

    #[tokio::test]
    async fn reload_endpoint_requires_admin_token_when_configured() {
        let mut config = EffectiveConfig::default();
        config.dashboard.allow_public = true;
        config.admin_token = Some("secret-token".to_string());
        let app = router(test_state(config));

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/config/reload")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"dry_run":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let authorized = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/config/reload")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .header("x-kubio-admin-token", "secret-token")
                    .body(Body::from(r#"{"dry_run":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(authorized.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn active_config_endpoint_includes_generation() {
        let app = router(test_state(EffectiveConfig::default()));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/config/active")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    fn test_state(config: EffectiveConfig) -> DashboardState {
        let config = Arc::new(config);
        DashboardState {
            config: config.clone(),
            observer: Arc::new(Observer::new(100, 100, 100, 2, 2, 1)),
            store: Arc::new(MemoryStore::new(&StorageConfig::default())),
            reloader: None,
        }
    }
}
