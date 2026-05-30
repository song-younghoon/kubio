use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use kubio_core::{
    DecisionReason, EffectiveConfig, Mode, RouteHintConfig, RouteMatchConfig, RouteQueryConfig,
    RouteSafetyConfig, RouteState,
};
use kubio_observe::{EventType, Observer};
use kubio_policy::PolicyEngine;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::{CacheStore, MemoryStore};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use url::Url;

#[tokio::test]
async fn watch_mode_forwards_and_observes() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Watch, 100, 5, 20).await;

    let body = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "stable");

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.origin_requests, 1);
    assert_eq!(snapshot.overview.reused_responses, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn authorization_is_protected_and_never_stored() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;
    let client = reqwest::Client::new();

    for _ in 0..3 {
        let response = client
            .get(format!("{}/auth", runtime.proxy_url()))
            .header("authorization", "Bearer secret")
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protected_requests, 3);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn sensitive_header_values_do_not_appear_in_snapshots_or_metrics() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/auth", runtime.proxy_url()))
        .header("authorization", "Bearer raw-secret-token")
        .header("cookie", "session=raw-cookie-secret")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let snapshot = runtime.observer.snapshot();
    let snapshot_json = serde_json::to_string(&snapshot).unwrap();
    let metrics = kubio_telemetry::render_metrics(&snapshot, &runtime.store.stats());

    for output in [snapshot_json.as_str(), metrics.as_str()] {
        assert!(!output.contains("raw-secret-token"));
        assert!(!output.contains("raw-cookie-secret"));
        assert!(!output.contains("Bearer"));
        assert!(!output.contains("session="));
    }

    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn cookie_is_protected_and_never_stored() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;
    let client = reqwest::Client::new();

    for _ in 0..3 {
        let response = client
            .get(format!("{}/cookie", runtime.proxy_url()))
            .header("cookie", "session=secret")
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protected_requests, 3);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn set_cookie_response_is_never_stored() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;

    for _ in 0..3 {
        let response = reqwest::get(format!("{}/set-cookie", runtime.proxy_url()))
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    assert_eq!(runtime.store.stats().entries, 0);
    assert!(runtime
        .observer
        .snapshot()
        .routes
        .iter()
        .any(|route| route.reasons.contains(&DecisionReason::HasSetCookie)));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn no_store_response_is_not_cached() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;

    for _ in 0..3 {
        let response = reqwest::get(format!("{}/nostore", runtime.proxy_url()))
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn private_and_no_cache_responses_are_not_cached() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;

    for path in ["/private", "/nocache"] {
        for _ in 0..3 {
            let response = reqwest::get(format!("{}{}", runtime.proxy_url(), path))
                .await
                .unwrap();
            assert!(response.status().is_success());
        }
    }

    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn vary_wildcard_response_is_not_cached() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;

    for _ in 0..3 {
        let response = reqwest::get(format!("{}/vary-star", runtime.proxy_url()))
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    assert_eq!(runtime.store.stats().entries, 0);
    assert!(runtime
        .observer
        .snapshot()
        .routes
        .iter()
        .any(|route| route.reasons.contains(&DecisionReason::VaryWildcard)));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn unsafe_method_forwards_body_and_is_not_reused() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;
    let client = reqwest::Client::new();

    for _ in 0..2 {
        let body = client
            .post(format!("{}/echo", runtime.proxy_url()))
            .body("posted")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "posted");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protected_requests, 2);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn auto_mode_reuses_after_shadow_confidence() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/stable", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "stable");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.origin_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 1);
    assert_eq!(runtime.store.stats().entries, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn shadow_mismatch_blocks_auto_reuse() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 2, 1, 1).await;

    for _ in 0..4 {
        let response = reqwest::get(format!("{}/unstable", runtime.proxy_url()))
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.reused_responses, 0);
    assert!(snapshot
        .routes
        .iter()
        .any(|route| route.state == RouteState::Protected
            && route.reasons.contains(&DecisionReason::ShadowMismatch)));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn origin_timeout_returns_gateway_timeout() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_with_origin_timeout(
        origin.url(),
        Mode::Watch,
        Duration::from_millis(25),
    )
    .await;

    let response = reqwest::get(format!("{}/slow", runtime.proxy_url()))
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    assert!(runtime
        .observer
        .snapshot()
        .events
        .iter()
        .any(|event| event.event_type == EventType::OriginRequestFailed));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn panic_switch_stops_and_restores_reuse() {
    let origin = TestOrigin::start().await;
    let panic_file = temp_panic_file();
    let runtime =
        TestRuntime::start_with_panic(origin.url(), Mode::Auto, 2, 2, 1, Some(panic_file.clone()))
            .await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/stable", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "stable");
    }
    assert_eq!(runtime.observer.snapshot().overview.reused_responses, 1);

    std::fs::write(&panic_file, "disabled").unwrap();
    for _ in 0..2 {
        let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
            .await
            .unwrap();
        assert!(response.status().is_success());
    }
    let active_snapshot = runtime.observer.snapshot();
    assert_eq!(active_snapshot.overview.reused_responses, 1);
    assert!(active_snapshot
        .routes
        .iter()
        .any(|route| route.reasons.contains(&DecisionReason::PanicSwitchActive)));
    assert_eq!(
        active_snapshot
            .events
            .iter()
            .filter(|event| event.event_type == EventType::PanicSwitchEnabled)
            .count(),
        1
    );

    std::fs::remove_file(&panic_file).unwrap();
    let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap();
    assert!(response.status().is_success());
    let restored_snapshot = runtime.observer.snapshot();
    assert_eq!(restored_snapshot.overview.reused_responses, 2);
    assert_eq!(
        restored_snapshot
            .events
            .iter()
            .filter(|event| event.event_type == EventType::PanicSwitchDisabled)
            .count(),
        1
    );

    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn stale_etag_entry_revalidates_with_304() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/etag", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "etag-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.revalidation_not_modified, 1);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn stale_last_modified_entry_revalidates_with_304() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/last-modified", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "last-modified-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.revalidation_not_modified, 1);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn no_cache_with_validator_revalidates_before_reuse() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/nocache-etag", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "nocache-etag-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.revalidation_not_modified, 1);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn unsafe_304_metadata_purges_stored_entry_and_refetches() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..2 {
        let body = reqwest::get(format!("{}/unsafe-304", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "unsafe-original");
    }

    let body = reqwest::get(format!("{}/unsafe-304", runtime.proxy_url()))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert_eq!(body, "unsafe-refresh");
    assert_eq!(runtime.observer.snapshot().overview.reused_responses, 0);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn stale_if_error_serves_verified_stale_response() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/stale-error", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "stale-error-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.stale_responses_served, 1);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn route_query_hint_ignores_configured_parameters() {
    let origin = TestOrigin::start().await;
    let hint = RouteHintConfig {
        name: Some("query test".to_string()),
        route_match: RouteMatchConfig {
            method: "GET".to_string(),
            path: "/query".to_string(),
        },
        query: RouteQueryConfig {
            include: Vec::new(),
            ignore: vec!["utm_*".to_string()],
        },
        ..route_hint_defaults("GET", "/query")
    };
    let runtime =
        TestRuntime::start_with_routes(origin.url(), Mode::Auto, 2, 2, 1, vec![hint]).await;

    for suffix in ["?utm_source=a", "?utm_source=b", "?utm_source=c"] {
        let body = reqwest::get(format!("{}/query{}", runtime.proxy_url(), suffix))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "query");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.origin_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 1);
    assert_eq!(snapshot.overview.route_hints_applied, 3);
    assert_eq!(snapshot.overview.query_hints_applied, 3);
    let query_route = snapshot
        .routes
        .iter()
        .find(|route| route.route_id.as_label() == "GET /query")
        .unwrap();
    assert_eq!(query_route.route_hint.as_deref(), Some("query test"));
    assert_eq!(query_route.route_hint_applied, 3);
    assert_eq!(query_route.query_hint_applied, 3);
    assert!(query_route
        .query_params
        .iter()
        .any(|param| param.name == "utm_source" && param.configured_action == "ignore"));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn route_query_hint_does_not_apply_to_non_matching_route() {
    let origin = TestOrigin::start().await;
    let hint = RouteHintConfig {
        name: Some("query test".to_string()),
        route_match: RouteMatchConfig {
            method: "GET".to_string(),
            path: "/query".to_string(),
        },
        query: RouteQueryConfig {
            include: Vec::new(),
            ignore: vec!["utm_*".to_string()],
        },
        ..route_hint_defaults("GET", "/query")
    };
    let runtime =
        TestRuntime::start_with_routes(origin.url(), Mode::Auto, 2, 2, 1, vec![hint]).await;

    for suffix in ["?utm_source=a", "?utm_source=b", "?utm_source=c"] {
        let body = reqwest::get(format!("{}/other-query{}", runtime.proxy_url(), suffix))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "other-query");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.reused_responses, 0);
    assert_eq!(snapshot.overview.route_hints_applied, 0);
    assert_eq!(snapshot.overview.query_hints_applied, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn route_and_query_hint_rejections_are_observed() {
    let origin = TestOrigin::start().await;
    let hint = RouteHintConfig {
        name: Some("auth query test".to_string()),
        route_match: RouteMatchConfig {
            method: "GET".to_string(),
            path: "/auth".to_string(),
        },
        query: RouteQueryConfig {
            include: Vec::new(),
            ignore: vec!["utm_*".to_string()],
        },
        ..route_hint_defaults("GET", "/auth")
    };
    let runtime =
        TestRuntime::start_with_routes(origin.url(), Mode::Auto, 2, 2, 1, vec![hint]).await;
    let response = reqwest::Client::new()
        .get(format!("{}/auth?utm_source=a", runtime.proxy_url()))
        .header("authorization", "Bearer secret")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.route_hints_rejected, 1);
    assert_eq!(snapshot.overview.query_hints_rejected, 1);
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.event_type == EventType::RouteHintRejected));
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.event_type == EventType::QueryHintRejected));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn force_protect_hint_prevents_reuse_of_safe_route() {
    let origin = TestOrigin::start().await;
    let hint = RouteHintConfig {
        name: Some("force protect stable".to_string()),
        route_match: RouteMatchConfig {
            method: "GET".to_string(),
            path: "/stable".to_string(),
        },
        safety: RouteSafetyConfig {
            force_protect: true,
            ..Default::default()
        },
        ..route_hint_defaults("GET", "/stable")
    };
    let runtime =
        TestRuntime::start_with_routes(origin.url(), Mode::Auto, 1, 1, 1, vec![hint]).await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/stable", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "stable");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.reused_responses, 0);
    assert_eq!(snapshot.overview.route_hints_applied, 3);
    assert!(snapshot
        .routes
        .iter()
        .any(|route| route.route_id.as_label() == "GET /stable"
            && route.reasons.contains(&DecisionReason::RouteHintApplied)));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn query_intelligence_tracks_cardinality_and_suggestions_without_values() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 100, 5, 20).await;

    for suffix in [
        "?utm_source=raw-campaign-a",
        "?utm_source=raw-campaign-b",
        "?utm_source=raw-campaign-c",
    ] {
        let body = reqwest::get(format!("{}/query-intel{}", runtime.proxy_url(), suffix))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "query-intel");
    }

    let snapshot = runtime.observer.snapshot();
    let snapshot_json = serde_json::to_string(&snapshot).unwrap();
    assert!(!snapshot_json.contains("raw-campaign"));
    let query_route = snapshot
        .routes
        .iter()
        .find(|route| route.route_id.as_label() == "GET /query-intel")
        .unwrap();
    let utm_source = query_route
        .query_params
        .iter()
        .find(|param| param.name == "utm_source")
        .unwrap();
    assert_eq!(utm_source.cardinality, "low");
    assert!(!utm_source.fingerprint_sensitive);
    assert_eq!(utm_source.suggestion.as_deref(), Some("candidate_ignore"));
    assert_eq!(snapshot.overview.query_param_suggestions, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn query_intelligence_marks_fingerprint_sensitive_params() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 100, 5, 20).await;

    for suffix in ["?variant=a", "?variant=b", "?variant=c"] {
        let body = reqwest::get(format!("{}/query-sensitive{}", runtime.proxy_url(), suffix))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(body.contains("variant="));
    }

    let snapshot = runtime.observer.snapshot();
    let query_route = snapshot
        .routes
        .iter()
        .find(|route| route.route_id.as_label() == "GET /query-sensitive")
        .unwrap();
    let variant = query_route
        .query_params
        .iter()
        .find(|param| param.name == "variant")
        .unwrap();
    assert!(variant.fingerprint_sensitive);
    assert_eq!(variant.suggestion, None);
    runtime.shutdown().await;
    origin.shutdown().await;
}

struct TestOrigin {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestOrigin {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/stable", get(|| async { "stable" }))
            .route("/auth", get(origin_counted))
            .route("/cookie", get(origin_counted))
            .route("/set-cookie", get(set_cookie))
            .route("/nostore", get(no_store))
            .route("/private", get(private))
            .route("/nocache", get(no_cache))
            .route("/nocache-etag", get(no_cache_etag))
            .route("/etag", get(etag))
            .route("/last-modified", get(last_modified))
            .route("/unsafe-304", get(unsafe_304))
            .route("/stale-error", get(stale_error))
            .route("/query", get(|| async { "query" }))
            .route("/other-query", get(|| async { "other-query" }))
            .route("/query-intel", get(|| async { "query-intel" }))
            .route("/query-sensitive", get(query_sensitive))
            .route("/vary-star", get(vary_star))
            .route("/unstable", get(unstable))
            .route("/slow", get(slow))
            .route("/echo", post(echo))
            .with_state(hits);
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Self {
            addr,
            shutdown: Some(tx),
        }
    }

    fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.addr)).unwrap()
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn origin_counted(State(hits): State<Arc<AtomicUsize>>) -> String {
    hits.fetch_add(1, Ordering::SeqCst);
    "auth".to_string()
}

async fn no_store() -> impl IntoResponse {
    ([("cache-control", "no-store")], "no-store")
}

async fn set_cookie() -> impl IntoResponse {
    ([("set-cookie", "session=secret")], "set-cookie")
}

async fn private() -> impl IntoResponse {
    ([("cache-control", "private")], "private")
}

async fn no_cache() -> impl IntoResponse {
    ([("cache-control", "no-cache")], "no-cache")
}

async fn no_cache_etag(headers: HeaderMap) -> impl IntoResponse {
    if headers.contains_key("if-none-match") {
        (
            axum::http::StatusCode::NOT_MODIFIED,
            [("etag", "\"nocache-v1\""), ("cache-control", "no-cache")],
            "",
        )
    } else {
        (
            axum::http::StatusCode::OK,
            [("etag", "\"nocache-v1\""), ("cache-control", "no-cache")],
            "nocache-etag-body",
        )
    }
}

async fn etag(headers: HeaderMap) -> impl IntoResponse {
    if headers.contains_key("if-none-match") {
        (
            axum::http::StatusCode::NOT_MODIFIED,
            [("etag", "\"etag-v1\""), ("cache-control", "max-age=0")],
            "",
        )
    } else {
        (
            axum::http::StatusCode::OK,
            [("etag", "\"etag-v1\""), ("cache-control", "max-age=0")],
            "etag-body",
        )
    }
}

async fn last_modified(headers: HeaderMap) -> impl IntoResponse {
    if headers.contains_key("if-modified-since") {
        (
            axum::http::StatusCode::NOT_MODIFIED,
            [
                ("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT"),
                ("cache-control", "max-age=0"),
            ],
            "",
        )
    } else {
        (
            axum::http::StatusCode::OK,
            [
                ("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT"),
                ("cache-control", "max-age=0"),
            ],
            "last-modified-body",
        )
    }
}

async fn unsafe_304(State(hits): State<Arc<AtomicUsize>>, headers: HeaderMap) -> impl IntoResponse {
    if headers.contains_key("if-none-match") {
        return (
            StatusCode::NOT_MODIFIED,
            [("etag", "\"unsafe-v1\""), ("cache-control", "no-store")],
            "",
        );
    }

    let count = hits.fetch_add(1, Ordering::SeqCst);
    if count < 2 {
        (
            StatusCode::OK,
            [("etag", "\"unsafe-v1\""), ("cache-control", "max-age=0")],
            "unsafe-original",
        )
    } else {
        (
            StatusCode::OK,
            [("etag", "\"unsafe-v2\""), ("cache-control", "no-store")],
            "unsafe-refresh",
        )
    }
}

async fn stale_error(headers: HeaderMap) -> impl IntoResponse {
    if headers.contains_key("if-none-match") {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            [("etag", "\"stale-v1\""), ("cache-control", "max-age=0")],
            "origin-error",
        )
    } else {
        (
            axum::http::StatusCode::OK,
            [
                ("etag", "\"stale-v1\""),
                ("cache-control", "max-age=0, stale-if-error=60"),
            ],
            "stale-error-body",
        )
    }
}

async fn query_sensitive(request: Request<Body>) -> impl IntoResponse {
    let query = request.uri().query().unwrap_or("");
    format!("query-sensitive:{query}")
}

async fn vary_star() -> impl IntoResponse {
    ([("vary", "*")], "vary")
}

async fn unstable(State(hits): State<Arc<AtomicUsize>>) -> String {
    let count = hits.fetch_add(1, Ordering::SeqCst);
    format!("unstable-{count}")
}

async fn slow() -> impl IntoResponse {
    tokio::time::sleep(Duration::from_millis(200)).await;
    "slow"
}

async fn echo(request: Request<Body>) -> impl IntoResponse {
    request.into_body()
}

struct TestRuntime {
    addr: SocketAddr,
    observer: Arc<Observer>,
    store: Arc<MemoryStore>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestRuntime {
    async fn start(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
    ) -> Self {
        Self::start_with_panic(
            origin,
            mode,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
            None,
        )
        .await
    }

    async fn start_with_panic(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
        panic_file: Option<PathBuf>,
    ) -> Self {
        Self::start_with_options(
            origin,
            mode,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
            panic_file,
            None,
        )
        .await
    }

    async fn start_with_origin_timeout(origin: Url, mode: Mode, origin_timeout: Duration) -> Self {
        Self::start_with_options(origin, mode, 100, 5, 20, None, Some(origin_timeout)).await
    }

    async fn start_with_routes(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
        routes: Vec<RouteHintConfig>,
    ) -> Self {
        Self::start_with_options_and_routes(
            origin,
            mode,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
            None,
            None,
            routes,
        )
        .await
    }

    async fn start_with_options(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
        panic_file: Option<PathBuf>,
        origin_timeout: Option<Duration>,
    ) -> Self {
        Self::start_with_options_and_routes(
            origin,
            mode,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
            panic_file,
            origin_timeout,
            Vec::new(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_with_options_and_routes(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
        panic_file: Option<PathBuf>,
        origin_timeout: Option<Duration>,
        routes: Vec<RouteHintConfig>,
    ) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut policy_config = defaults.policy.clone();
        policy_config.min_route_samples = min_route_samples;
        policy_config.min_key_repeats = min_key_repeats;
        policy_config.min_shadow_validations = min_shadow_validations;
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        if let Some(origin_timeout) = origin_timeout {
            server_config.origin_timeout = origin_timeout;
        }
        let config = EffectiveConfig {
            origin,
            mode,
            server: server_config,
            policy: policy_config,
            panic_file,
            routes,
            ..defaults
        };

        let config = Arc::new(config);
        let observer = Arc::new(Observer::new(
            100,
            100,
            100,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
        ));
        let store = Arc::new(MemoryStore::new(&config.storage));
        let policy = Arc::new(PolicyEngine::new(&config));
        let state = ProxyState::new(config, policy, observer.clone(), store.clone()).unwrap();
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = run_proxy(state, async {
                let _ = rx.await;
            })
            .await;
        });

        let runtime = Self {
            addr,
            observer,
            store,
            shutdown: Some(tx),
        };
        runtime.wait_ready().await;
        runtime
    }

    fn proxy_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn wait_ready(&self) {
        for _ in 0..50 {
            if TcpStream::connect(self.addr).await.is_ok() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn unused_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn temp_panic_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "kubio-panic-switch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    path
}

fn route_hint_defaults(method: &str, path: &str) -> RouteHintConfig {
    RouteHintConfig {
        name: None,
        route_match: RouteMatchConfig {
            method: method.to_string(),
            path: path.to_string(),
        },
        freshness: Default::default(),
        query: Default::default(),
        vary: Default::default(),
        stale_if_error: Default::default(),
        safety: Default::default(),
    }
}
