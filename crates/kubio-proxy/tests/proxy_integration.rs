use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
#[cfg(feature = "experimental-http3")]
use bytes::{Buf, Bytes, BytesMut};
#[cfg(feature = "experimental-http3")]
use kubio_core::TlsConfig;
use kubio_core::{
    DecisionReason, EffectiveConfig, Mode, OriginProtocolPreference, RouteHintConfig,
    RouteMatchConfig, RouteQueryConfig, RouteSafetyConfig, RouteState,
};
use kubio_observe::{EventType, Observer};
use kubio_policy::PolicyEngine;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::{CacheStore, MemoryStore};
#[cfg(feature = "experimental-http3")]
use kubio_transport::{serve_http3_router, Http3ServerTelemetry};
#[cfg(feature = "experimental-http3")]
use quinn::crypto::rustls::QuicClientConfig;
#[cfg(feature = "experimental-http3")]
use std::fs::File;
#[cfg(feature = "experimental-http3")]
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use url::Url;

static NEXT_TEST_PORT: AtomicUsize = AtomicUsize::new(30000);
#[cfg(feature = "experimental-http3")]
static NEXT_TEST_UDP_PORT: AtomicUsize = AtomicUsize::new(40000);

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
async fn h2c_prior_knowledge_forwards_and_observes() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_h2c(origin.url(), Mode::Watch, 100, 5, 20).await;
    let client = h2_client();

    let response = client
        .get(format!("{}/stable", runtime.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.version(), reqwest::Version::HTTP_2);
    assert_eq!(response.text().await.unwrap(), "stable");

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.origin_requests, 1);
    assert_eq!(snapshot.overview.reused_responses, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn h2c_auto_mode_reuses_after_shadow_confidence() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_h2c(origin.url(), Mode::Auto, 2, 2, 1).await;
    let client = h2_client();

    for _ in 0..3 {
        let response = client
            .get(format!("{}/stable", runtime.proxy_url()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.version(), reqwest::Version::HTTP_2);
        assert_eq!(response.text().await.unwrap(), "stable");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.downstream_http2_requests, 3);
    assert_eq!(snapshot.overview.origin_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 1);
    assert_eq!(runtime.store.stats().entries, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn h2c_authorization_and_cookie_are_protected() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_h2c(origin.url(), Mode::Auto, 1, 1, 1).await;
    let client = h2_client();

    let auth = client
        .get(format!("{}/auth", runtime.proxy_url()))
        .header("authorization", "Bearer secret")
        .send()
        .await
        .unwrap();
    assert_eq!(auth.version(), reqwest::Version::HTTP_2);
    assert!(auth.status().is_success());

    let cookie = client
        .get(format!("{}/cookie", runtime.proxy_url()))
        .header("cookie", "session=secret")
        .send()
        .await
        .unwrap();
    assert_eq!(cookie.version(), reqwest::Version::HTTP_2);
    assert!(cookie.status().is_success());

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protected_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 0);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn h2c_unsafe_response_headers_are_not_stored() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_h2c(origin.url(), Mode::Auto, 1, 1, 1).await;
    let client = h2_client();

    for path in ["set-cookie", "nostore", "private"] {
        let response = client
            .get(format!("{}/{}", runtime.proxy_url(), path))
            .send()
            .await
            .unwrap();
        assert_eq!(response.version(), reqwest::Version::HTTP_2);
        assert!(response.status().is_success());
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.reused_responses, 0);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn h2c_stale_etag_entry_revalidates_with_304() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_h2c(origin.url(), Mode::Auto, 2, 2, 1).await;
    let client = h2_client();

    for _ in 0..3 {
        let response = client
            .get(format!("{}/etag", runtime.proxy_url()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.version(), reqwest::Version::HTTP_2);
        assert_eq!(response.text().await.unwrap(), "etag-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.revalidation_not_modified, 1);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn h2c_stale_if_error_serves_verified_stale_response() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_h2c(origin.url(), Mode::Auto, 2, 2, 1).await;
    let client = h2_client();

    for _ in 0..3 {
        let response = client
            .get(format!("{}/stale-error", runtime.proxy_url()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.version(), reqwest::Version::HTTP_2);
        assert_eq!(response.text().await.unwrap(), "stale-error-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.stale_responses_served, 1);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn h1_and_h2c_share_safe_cache_keys() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_h2c(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..2 {
        let body = reqwest::get(format!("{}/stable", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "stable");
    }

    let response = h2_client()
        .get(format!("{}/stable", runtime.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.version(), reqwest::Version::HTTP_2);
    assert_eq!(response.text().await.unwrap(), "stable");

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.downstream_http1_requests, 2);
    assert_eq!(snapshot.overview.downstream_http2_requests, 1);
    assert_eq!(snapshot.overview.origin_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn h3_safe_get_reuses_after_shadow_confidence() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let response = h3_get(&runtime, "/stable").await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, "stable");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.downstream_http3_requests, 3);
    assert_eq!(snapshot.overview.origin_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 1);
    assert_eq!(runtime.store.stats().entries, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn h3_authorization_and_cookie_are_protected() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3(origin.url(), Mode::Auto, 1, 1, 1).await;

    let auth = h3_get_with_headers(&runtime, "/auth", &[("authorization", "Bearer secret")]).await;
    assert_eq!(auth.status, StatusCode::OK);

    let cookie = h3_get_with_headers(&runtime, "/cookie", &[("cookie", "session=secret")]).await;
    assert_eq!(cookie.status, StatusCode::OK);

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protected_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 0);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn h3_unsafe_response_headers_are_not_stored() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3(origin.url(), Mode::Auto, 1, 1, 1).await;

    for path in ["/set-cookie", "/nostore", "/private"] {
        let response = h3_get(&runtime, path).await;
        assert_eq!(response.status, StatusCode::OK);
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.reused_responses, 0);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn h3_stale_etag_entry_revalidates_with_304() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let response = h3_get(&runtime, "/etag").await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, "etag-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.revalidation_not_modified, 1);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn h3_stale_if_error_serves_verified_stale_response() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let response = h3_get(&runtime, "/stale-error").await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, "stale-error-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.stale_responses_served, 1);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn h1_and_h3_share_safe_cache_keys() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3(origin.url(), Mode::Auto, 2, 2, 1).await;
    let client = https_client();

    for _ in 0..2 {
        let body = client
            .get(format!("{}/stable", runtime.proxy_https_url()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "stable");
    }

    let response = h3_get_with_headers(&runtime, "/stable", &[("accept", "*/*")]).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, "stable");

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.downstream_http1_requests, 2);
    assert_eq!(snapshot.overview.downstream_http3_requests, 1);
    assert_eq!(snapshot.overview.origin_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn alt_svc_is_emitted_for_configured_authority() {
    let origin = TestOrigin::start().await;
    let runtime =
        TestRuntime::start_http3_advertising_to_localhost(origin.url(), Mode::Watch).await;
    let response = https_client()
        .get(format!("{}/stable", runtime.proxy_https_url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get("alt-svc").unwrap().to_str().unwrap(),
        format!("h3=\":{}\"; ma=3600", runtime.http3_addr().port())
    );
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.alt_svc.advertised, 1);
    let metrics = kubio_telemetry::render_metrics(&snapshot, &runtime.store.stats());
    assert!(metrics.contains(
        "kubio_alt_svc_advertisements_total{outcome=\"advertised\",reason=\"configured_authority\"} 1"
    ));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn alt_svc_is_skipped_for_unconfigured_authority() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3_advertising_authorities(
        origin.url(),
        Mode::Watch,
        vec!["example.com".to_string()],
    )
    .await;
    let response = https_client()
        .get(format!("{}/stable", runtime.proxy_https_url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(!response.headers().contains_key("alt-svc"));
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.alt_svc.skipped_authority_not_allowed, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn alt_svc_requires_exact_authority_match() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3_advertising_authorities(
        origin.url(),
        Mode::Watch,
        vec!["localhost".to_string()],
    )
    .await;
    let response = https_client()
        .get(format!("{}/stable", runtime.proxy_https_url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(!response.headers().contains_key("alt-svc"));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn origin_alt_svc_is_not_forwarded_for_unconfigured_authority() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_http3(origin.url(), Mode::Watch, 100, 5, 20).await;
    let response = https_client()
        .get(format!("{}/origin-alt-svc", runtime.proxy_https_url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(!response.headers().contains_key("alt-svc"));
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
async fn backpressure_limit_rejects_new_requests() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_with_max_in_flight(origin.url(), Mode::Watch, 1).await;
    let client = reqwest::Client::new();
    let slow_url = format!("{}/slow", runtime.proxy_url());

    let first_client = client.clone();
    let first_url = slow_url.clone();
    let first = tokio::spawn(async move { first_client.get(first_url).send().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(25)).await;

    let rejected = client.get(slow_url).send().await.unwrap();
    assert_eq!(rejected.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert!(first.await.unwrap().status().is_success());

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.backpressure_rejections, 1);
    assert_eq!(snapshot.overview.max_in_flight_requests, 1);
    assert_eq!(snapshot.overview.in_flight_requests, 0);
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.event_type == EventType::BackpressureRejected));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn h2c_header_list_limit_rejects_oversized_headers() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_h2c_with_header_limit(origin.url(), 32).await;
    let client = h2_client();

    let response = client
        .get(format!("{}/stable", runtime.proxy_url()))
        .header("x-large-header", "x".repeat(128))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
    );
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.origin_requests, 0);
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.event_type == EventType::RequestHeaderLimitExceeded));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn large_protected_response_is_not_stored() {
    let origin = TestOrigin::start().await;
    let runtime =
        TestRuntime::start_with_small_response_limits(origin.url(), "/large-private").await;

    let body = reqwest::get(format!("{}/large-private", runtime.proxy_url()))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert_eq!(body.len(), LARGE_BODY_SIZE);
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protected_requests, 1);
    assert_eq!(runtime.store.stats().entries, 0);
    assert!(!snapshot
        .events
        .iter()
        .any(|event| event.event_type == EventType::StoreSaturated));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn oversized_storeable_response_is_not_partially_stored() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_with_small_response_limits(origin.url(), "/large-cache").await;

    for _ in 0..2 {
        let body = reqwest::get(format!("{}/large-cache", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body.len(), LARGE_BODY_SIZE);
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.reused_responses, 0);
    assert_eq!(runtime.store.stats().entries, 0);
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.event_type == EventType::StoreSaturated));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn origin_protocol_fallback_is_recorded_when_preference_is_not_met() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_with_origin_protocol(
        origin.url(),
        OriginProtocolPreference::Http3,
        true,
    )
    .await;

    let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap();
    assert!(response.status().is_success());

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protocol_fallbacks, 1);
    assert_eq!(snapshot.overview.upstream_http1_requests, 1);
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.event_type == EventType::ProtocolFallback));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn h2_prior_knowledge_retries_http1_when_fallback_is_enabled() {
    let origin = TestHttp1OnlyOrigin::start().await;
    let runtime = TestRuntime::start_with_origin_protocol(
        origin.url(),
        OriginProtocolPreference::Http2,
        true,
    )
    .await;

    let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap();

    assert!(response.status().is_success());
    assert_eq!(response.text().await.unwrap(), "h1-only");
    assert_eq!(origin.successful_hits(), 1);
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protocol_fallbacks, 1);
    assert_eq!(snapshot.overview.upstream_http1_requests, 1);
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.event_type == EventType::ProtocolFallback));
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn required_origin_protocol_mismatch_fails_closed() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start_with_origin_protocol(
        origin.url(),
        OriginProtocolPreference::Http3,
        false,
    )
    .await;

    let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.origin_requests, 1);
    assert_eq!(snapshot.overview.protocol_fallbacks, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn upstream_h3_origin_success_records_protocol() {
    let origin = TestHttp3Origin::start().await;
    let runtime = TestRuntime::start_with_upstream_http3(
        origin.url(),
        OriginProtocolPreference::Http3,
        false,
    )
    .await;

    let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap();

    assert!(response.status().is_success());
    assert_eq!(response.text().await.unwrap(), "stable");
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.upstream_http3_requests, 1);
    assert_eq!(snapshot.overview.upstream_http3.attempts, 1);
    assert_eq!(snapshot.overview.upstream_http3.successes, 1);
    assert_eq!(snapshot.overview.protocol_fallbacks, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn upstream_h3_required_failure_fails_closed() {
    let unused = unused_udp_addr();
    let origin = Url::parse(&format!("https://localhost:{}", unused.port())).unwrap();
    let runtime =
        TestRuntime::start_with_upstream_http3(origin, OriginProtocolPreference::Http3, false)
            .await;

    let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.upstream_http3.attempts, 1);
    assert_eq!(snapshot.overview.upstream_http3.failures, 1);
    assert_eq!(snapshot.overview.upstream_http3.required_failures, 1);
    runtime.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn upstream_h3_preferred_falls_back_for_replayable_http_origin() {
    let origin = TestOrigin::start().await;
    let runtime =
        TestRuntime::start_with_upstream_http3(origin.url(), OriginProtocolPreference::Http3, true)
            .await;

    let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap();

    assert!(response.status().is_success());
    assert_eq!(response.text().await.unwrap(), "stable");
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.upstream_http3.skipped_not_https, 1);
    assert_eq!(snapshot.overview.upstream_http3.fallbacks, 1);
    assert_eq!(snapshot.overview.protocol_fallbacks, 1);
    assert_eq!(snapshot.overview.upstream_http1_requests, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn upstream_h3_blocks_non_replayable_fallback() {
    let origin = TestOrigin::start().await;
    let runtime =
        TestRuntime::start_with_upstream_http3(origin.url(), OriginProtocolPreference::Http3, true)
            .await;

    let response = reqwest::Client::new()
        .post(format!("{}/echo", runtime.proxy_url()))
        .body("unsafe-body")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.upstream_http3.skipped_non_replayable, 1);
    assert_eq!(snapshot.overview.upstream_http3.fallbacks, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[cfg(feature = "experimental-http3")]
#[tokio::test]
async fn revalidation_can_use_upstream_h3() {
    let origin = TestHttp3Origin::start().await;
    let runtime = TestRuntime::start_with_upstream_http3_auto(origin.url()).await;

    for _ in 0..3 {
        let response = reqwest::get(format!("{}/etag", runtime.proxy_url()))
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.text().await.unwrap(), "etag-body");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.revalidation_not_modified, 1);
    assert_eq!(snapshot.overview.upstream_http3_requests, 3);
    assert_eq!(snapshot.overview.upstream_http3.successes, 3);
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

fn h2_client() -> reqwest::Client {
    reqwest::Client::builder()
        .http2_prior_knowledge()
        .build()
        .unwrap()
}

#[cfg(feature = "experimental-http3")]
fn https_client() -> reqwest::Client {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
}

#[cfg(feature = "experimental-http3")]
struct H3TestResponse {
    status: StatusCode,
    body: String,
}

#[cfg(feature = "experimental-http3")]
async fn h3_get(runtime: &TestRuntime, path: &str) -> H3TestResponse {
    h3_get_with_headers(runtime, path, &[]).await
}

#[cfg(feature = "experimental-http3")]
async fn h3_get_with_headers(
    runtime: &TestRuntime,
    path: &str,
    headers: &[(&str, &str)],
) -> H3TestResponse {
    let mut last_error = None;
    for _ in 0..50 {
        match try_h3_get(runtime, path, headers).await {
            Ok(response) => return response,
            Err(err) => {
                last_error = Some(err);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
    panic!(
        "HTTP/3 request failed after retries: {}",
        last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "unknown error".to_string())
    );
}

#[cfg(feature = "experimental-http3")]
async fn try_h3_get(
    runtime: &TestRuntime,
    path: &str,
    headers: &[(&str, &str)],
) -> anyhow::Result<H3TestResponse> {
    let mut client = h3_client(runtime.http3_addr()).await?;
    let uri = format!("https://localhost:{}{path}", runtime.http3_addr().port());
    let mut request = Request::get(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut stream = client.send.send_request(request.body(())?).await?;
    stream.finish().await?;

    let response = stream.recv_response().await?;
    let status = response.status();
    let mut body = BytesMut::new();
    while let Some(mut chunk) = stream.recv_data().await? {
        let len = chunk.remaining();
        body.extend_from_slice(&chunk.copy_to_bytes(len));
    }
    drop(client.send);
    client.endpoint.close(0_u32.into(), b"done");
    client.driver.abort();
    Ok(H3TestResponse {
        status,
        body: String::from_utf8(body.to_vec())?,
    })
}

#[cfg(feature = "experimental-http3")]
struct H3TestClient {
    send: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    endpoint: quinn::Endpoint,
    driver: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "experimental-http3")]
async fn h3_client(addr: SocketAddr) -> anyhow::Result<H3TestClient> {
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    endpoint.set_default_client_config(h3_quinn_client_config()?);
    let connection = endpoint.connect(addr, "localhost")?.await?;
    let quic = h3_quinn::Connection::new(connection);
    let (mut connection, send) = h3::client::builder().build(quic).await?;
    let driver = tokio::spawn(async move {
        let _ = connection.wait_idle().await;
    });
    Ok(H3TestClient {
        send,
        endpoint,
        driver,
    })
}

#[cfg(feature = "experimental-http3")]
fn h3_quinn_client_config() -> anyhow::Result<quinn::ClientConfig> {
    let mut roots = quinn::rustls::RootCertStore::empty();
    for cert in test_tls_certs()? {
        roots.add(cert)?;
    }
    let mut tls = quinn::rustls::ClientConfig::builder_with_provider(Arc::new(
        quinn::rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&quinn::rustls::version::TLS13])?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    Ok(quinn::ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(tls)?,
    )))
}

#[cfg(feature = "experimental-http3")]
fn test_tls_certs() -> anyhow::Result<Vec<quinn::rustls::pki_types::CertificateDer<'static>>> {
    let file = File::open(test_tls_cert_path())?;
    rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

#[cfg(feature = "experimental-http3")]
fn test_tls_cert_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/localhost-cert.pem")
}

#[cfg(feature = "experimental-http3")]
fn test_tls_key_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/localhost-key.pem")
}

const LARGE_BODY_SIZE: usize = 8192;

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
            .route("/origin-alt-svc", get(origin_alt_svc))
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
            .route("/large-private", get(large_private))
            .route("/large-cache", get(large_cache))
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

struct TestHttp1OnlyOrigin {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestHttp1OnlyOrigin {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let task_hits = hits.clone();
        let (tx, mut rx) = oneshot::channel();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else {
                            continue;
                        };
                        let hits = task_hits.clone();
                        tokio::spawn(async move {
                            let mut buffer = [0; 1024];
                            let Ok(read) = stream.read(&mut buffer).await else {
                                return;
                            };
                            if buffer[..read].starts_with(b"PRI * HTTP/2.0") {
                                return;
                            }
                            hits.fetch_add(1, Ordering::SeqCst);
                            let response = concat!(
                                "HTTP/1.1 200 OK\r\n",
                                "content-length: 7\r\n",
                                "cache-control: public, max-age=60\r\n",
                                "connection: close\r\n",
                                "\r\n",
                                "h1-only",
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                        });
                    }
                }
            }
        });

        Self {
            addr,
            hits,
            shutdown: Some(tx),
        }
    }

    fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.addr)).unwrap()
    }

    fn successful_hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(feature = "experimental-http3")]
struct TestHttp3Origin {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

#[cfg(feature = "experimental-http3")]
impl TestHttp3Origin {
    async fn start() -> Self {
        let addr = unused_udp_addr();
        let defaults = EffectiveConfig::default();
        let mut server_config = defaults.server.clone();
        server_config.listen = unused_addr().await;
        server_config.tls = Some(TlsConfig {
            cert: test_tls_cert_path(),
            key: test_tls_key_path(),
        });
        server_config.http3.enabled = true;
        server_config.http3.listen = Some(addr);
        let config = Arc::new(EffectiveConfig {
            server: server_config,
            ..defaults
        });
        let app = Router::new()
            .route("/stable", get(|| async { "stable" }))
            .route("/etag", get(etag));
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = serve_http3_router(config, app, Http3ServerTelemetry::default(), async {
                let _ = rx.await;
            })
            .await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        Self {
            addr,
            shutdown: Some(tx),
        }
    }

    fn url(&self) -> Url {
        Url::parse(&format!("https://localhost:{}", self.addr.port())).unwrap()
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

async fn origin_alt_svc() -> impl IntoResponse {
    ([("alt-svc", "h3=\":443\"; ma=60")], "origin-alt-svc")
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

async fn large_private() -> impl IntoResponse {
    ([("cache-control", "private")], "x".repeat(LARGE_BODY_SIZE))
}

async fn large_cache() -> impl IntoResponse {
    (
        [("cache-control", "public, max-age=60")],
        "x".repeat(LARGE_BODY_SIZE),
    )
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
    #[cfg(feature = "experimental-http3")]
    http3_addr: Option<SocketAddr>,
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

    async fn start_with_max_in_flight(
        origin: Url,
        mode: Mode,
        max_in_flight_requests: usize,
    ) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        let mut performance = defaults.performance.clone();
        performance.max_in_flight_requests = max_in_flight_requests;
        let config = Arc::new(EffectiveConfig {
            origin,
            mode,
            server: server_config,
            performance,
            ..defaults
        });
        let observer = Arc::new(Observer::new(
            100,
            100,
            100,
            config.policy.min_route_samples,
            config.policy.min_key_repeats,
            config.policy.min_shadow_validations,
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
            #[cfg(feature = "experimental-http3")]
            http3_addr: None,
            observer,
            store,
            shutdown: Some(tx),
        };
        runtime.wait_ready().await;
        runtime
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

    async fn start_h2c(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
    ) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut policy_config = defaults.policy.clone();
        policy_config.min_route_samples = min_route_samples;
        policy_config.min_key_repeats = min_key_repeats;
        policy_config.min_shadow_validations = min_shadow_validations;
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        server_config.protocols.http2 = true;
        server_config.protocols.h2c = true;
        let config = Arc::new(EffectiveConfig {
            origin,
            mode,
            server: server_config,
            policy: policy_config,
            ..defaults
        });
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
            #[cfg(feature = "experimental-http3")]
            http3_addr: None,
            observer,
            store,
            shutdown: Some(tx),
        };
        runtime.wait_ready().await;
        runtime
    }

    #[cfg(feature = "experimental-http3")]
    async fn start_http3(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
    ) -> Self {
        Self::start_http3_with_advertise(
            origin,
            mode,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
            false,
            Vec::new(),
        )
        .await
    }

    #[cfg(feature = "experimental-http3")]
    async fn start_http3_advertising_to_localhost(origin: Url, mode: Mode) -> Self {
        Self::start_http3_with_advertise(origin, mode, 100, 5, 20, true, Vec::new()).await
    }

    #[cfg(feature = "experimental-http3")]
    async fn start_http3_advertising_authorities(
        origin: Url,
        mode: Mode,
        authorities: Vec<String>,
    ) -> Self {
        Self::start_http3_with_advertise(origin, mode, 100, 5, 20, true, authorities).await
    }

    #[cfg(feature = "experimental-http3")]
    async fn start_http3_with_advertise(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
        advertise: bool,
        authorities: Vec<String>,
    ) -> Self {
        let addr = unused_addr().await;
        let http3_addr = unused_udp_addr();
        let defaults = EffectiveConfig::default();
        let mut policy_config = defaults.policy.clone();
        policy_config.min_route_samples = min_route_samples;
        policy_config.min_key_repeats = min_key_repeats;
        policy_config.min_shadow_validations = min_shadow_validations;
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        server_config.tls = Some(TlsConfig {
            cert: test_tls_cert_path(),
            key: test_tls_key_path(),
        });
        server_config.http3.enabled = true;
        server_config.http3.listen = Some(http3_addr);
        server_config.http3.advertise = advertise;
        server_config.http3.authorities = if authorities.is_empty() && advertise {
            vec![format!("localhost:{}", addr.port())]
        } else {
            authorities
        };
        let config = Arc::new(EffectiveConfig {
            origin,
            mode,
            server: server_config,
            policy: policy_config,
            ..defaults
        });
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
            http3_addr: Some(http3_addr),
            observer,
            store,
            shutdown: Some(tx),
        };
        runtime.wait_ready().await;
        runtime
    }

    async fn start_h2c_with_header_limit(origin: Url, max_header_list_size: u64) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        server_config.protocols.http2 = true;
        server_config.protocols.h2c = true;
        server_config.http2.max_header_list_size = max_header_list_size;
        Self::start_from_config(EffectiveConfig {
            origin,
            server: server_config,
            ..defaults
        })
        .await
    }

    async fn start_with_small_response_limits(origin: Url, _path: &str) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        let mut policy_config = defaults.policy.clone();
        policy_config.min_route_samples = 1;
        policy_config.min_key_repeats = 1;
        policy_config.min_shadow_validations = 1;
        policy_config.max_object_size = 128;
        policy_config.max_fingerprint_body_size = 128;
        let mut storage_config = defaults.storage.clone();
        storage_config.max_object_size = 128;
        let mut performance = defaults.performance.clone();
        performance.max_buffered_response_size = 128;
        Self::start_from_config(EffectiveConfig {
            origin,
            mode: Mode::Auto,
            server: server_config,
            policy: policy_config,
            storage: storage_config,
            performance,
            ..defaults
        })
        .await
    }

    async fn start_with_origin_protocol(
        origin: Url,
        preferred: OriginProtocolPreference,
        fallback: bool,
    ) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        let mut origin_protocol = defaults.origin_protocol.clone();
        origin_protocol.preferred = preferred;
        origin_protocol.fallback = fallback;
        Self::start_from_config(EffectiveConfig {
            origin,
            server: server_config,
            origin_protocol,
            ..defaults
        })
        .await
    }

    #[cfg(feature = "experimental-http3")]
    async fn start_with_upstream_http3(
        origin: Url,
        preferred: OriginProtocolPreference,
        fallback: bool,
    ) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        server_config.origin_timeout = Duration::from_millis(250);
        let mut origin_protocol = defaults.origin_protocol.clone();
        origin_protocol.preferred = preferred;
        origin_protocol.fallback = fallback;
        origin_protocol.http3_experimental = true;
        origin_protocol.http3_ca_certs = vec![test_tls_cert_path()];
        Self::start_from_config(EffectiveConfig {
            origin,
            server: server_config,
            origin_protocol,
            ..defaults
        })
        .await
    }

    #[cfg(feature = "experimental-http3")]
    async fn start_with_upstream_http3_auto(origin: Url) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut server_config = defaults.server.clone();
        server_config.listen = addr;
        server_config.origin_timeout = Duration::from_millis(500);
        let mut origin_protocol = defaults.origin_protocol.clone();
        origin_protocol.preferred = OriginProtocolPreference::Http3;
        origin_protocol.fallback = false;
        origin_protocol.http3_experimental = true;
        origin_protocol.http3_ca_certs = vec![test_tls_cert_path()];
        let mut policy_config = defaults.policy.clone();
        policy_config.min_route_samples = 2;
        policy_config.min_key_repeats = 2;
        policy_config.min_shadow_validations = 1;
        Self::start_from_config(EffectiveConfig {
            origin,
            mode: Mode::Auto,
            server: server_config,
            origin_protocol,
            policy: policy_config,
            ..defaults
        })
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
            #[cfg(feature = "experimental-http3")]
            http3_addr: None,
            observer,
            store,
            shutdown: Some(tx),
        };
        runtime.wait_ready().await;
        runtime
    }

    async fn start_from_config(config: EffectiveConfig) -> Self {
        let addr = config.server.listen;
        let config = Arc::new(config);
        let observer = Arc::new(Observer::new(
            100,
            100,
            100,
            config.policy.min_route_samples,
            config.policy.min_key_repeats,
            config.policy.min_shadow_validations,
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
            #[cfg(feature = "experimental-http3")]
            http3_addr: None,
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

    #[cfg(feature = "experimental-http3")]
    fn proxy_https_url(&self) -> String {
        format!("https://localhost:{}", self.addr.port())
    }

    #[cfg(feature = "experimental-http3")]
    fn http3_addr(&self) -> SocketAddr {
        self.http3_addr.expect("HTTP/3 test runtime")
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
    for _ in 0..10_000 {
        let port = NEXT_TEST_PORT.fetch_add(1, Ordering::SeqCst);
        let port = 30000 + (port % 20000);
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        if let Ok(listener) = TcpListener::bind(addr).await {
            drop(listener);
            return addr;
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

#[cfg(feature = "experimental-http3")]
fn unused_udp_addr() -> SocketAddr {
    for _ in 0..10_000 {
        let port = NEXT_TEST_UDP_PORT.fetch_add(1, Ordering::SeqCst);
        let port = 40000 + (port % 20000);
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        if let Ok(socket) = std::net::UdpSocket::bind(addr) {
            drop(socket);
            return addr;
        }
    }
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    drop(socket);
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
