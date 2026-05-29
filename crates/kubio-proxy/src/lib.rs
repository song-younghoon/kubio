//! HTTP reverse proxy runtime for kubio.

use anyhow::Context;
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use http::header;
use kubio_core::{
    body_hash, build_cache_key, is_hop_by_hop_header, stable_header_hash, CacheKeyHash, Decision,
    DecisionReason, EffectiveConfig, Mode, ResponseFingerprint, RouteId,
};
use kubio_observe::{EventType, ObservationRecord, Observer};
use kubio_policy::PolicyEngine;
use kubio_store::{CacheEntry, CacheStore};
use reqwest::Client;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::net::TcpListener;
use tracing::{debug, warn};
use url::Url;

const DEFAULT_VARY_HEADERS: &[&str] = &["accept", "accept-encoding", "accept-language"];

#[derive(Clone)]
pub struct ProxyState {
    pub config: Arc<EffectiveConfig>,
    pub policy: Arc<PolicyEngine>,
    pub observer: Arc<Observer>,
    pub store: Arc<dyn CacheStore>,
    pub client: Client,
    panic_switch_was_active: Arc<AtomicBool>,
}

impl ProxyState {
    pub fn new(
        config: Arc<EffectiveConfig>,
        policy: Arc<PolicyEngine>,
        observer: Arc<Observer>,
        store: Arc<dyn CacheStore>,
    ) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("build origin HTTP client")?;
        Ok(Self {
            config,
            policy,
            observer,
            store,
            client,
            panic_switch_was_active: Arc::new(AtomicBool::new(false)),
        })
    }
}

pub fn router(state: ProxyState) -> Router {
    Router::new()
        .route("/{*path}", any(proxy_handler))
        .fallback(proxy_handler)
        .with_state(state)
}

pub async fn run_proxy<F>(state: ProxyState, shutdown: F) -> anyhow::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind(state.config.server.listen).await?;
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

async fn proxy_handler(State(state): State<ProxyState>, request: Request<Body>) -> Response<Body> {
    let started = std::time::Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(ToOwned::to_owned);
    let route_id = RouteId::from_method_path(&method, &path);
    let headers = request.headers().clone();
    let panic_active = panic_switch_active(state.config.panic_file.as_deref());
    record_panic_switch_transition(&state, panic_active, &route_id, None);

    let body_limit = state.config.policy.max_request_body_size;
    let body = match to_bytes(request.into_body(), body_limit).await {
        Ok(body) => body,
        Err(err) => {
            warn!(error = %err, "request body exceeded proxy buffer limit");
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
    };

    let mut request_signals = state
        .policy
        .request_signals(&method, &path, &headers, body.len());
    request_signals.query_param_count = query
        .as_deref()
        .map(count_query_params)
        .unwrap_or_default()
        .min(u16::MAX as usize) as u16;

    let route_state = state.observer.route_state(&route_id);
    let request_decision = state.policy.decide_request(
        state.config.mode,
        route_state,
        &request_signals,
        panic_active,
    );

    let cache_key_hash = if request_signals.method_cacheable {
        Some(
            build_cache_key(
                &method,
                state.config.origin.scheme(),
                &origin_authority(&state.config.origin),
                &path,
                query.as_deref(),
                &headers,
                DEFAULT_VARY_HEADERS,
            )
            .hash(),
        )
    } else {
        None
    };

    if state.config.mode == Mode::Auto
        && request_decision.decision != Decision::Protect
        && !panic_active
    {
        if let Some(key_hash) = cache_key_hash.as_ref() {
            if state.observer.is_auto_eligible(&route_id, key_hash) {
                match state.store.get(key_hash).await {
                    Ok(Some(entry)) if entry.is_fresh() => {
                        debug!(route = %route_id, "serving reused response");
                        state.observer.record_reuse(
                            route_id,
                            key_hash.clone(),
                            entry.status,
                            started.elapsed(),
                        );
                        return response_from_cache_entry(&state.config, entry);
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!(error = %err, "cache lookup failed; passing through to origin");
                        state.observer.push_event(
                            EventType::StoreErrorFailOpen,
                            Some(route_id.clone()),
                            Some(key_hash.clone()),
                            vec![DecisionReason::StoreError],
                            "cache lookup failed; passed through to origin",
                        );
                    }
                }
            }
        }
    }

    let origin_response = match send_origin(&state, &method, &uri, &headers, body.clone()).await {
        Ok(response) => response,
        Err(err) => {
            warn!(error = %err, "origin request failed");
            let status = if err.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            };
            state.observer.record(ObservationRecord {
                route_id,
                cache_key_hash,
                decision: Decision::Bypass,
                reasons: vec![DecisionReason::PolicyError],
                status: status.as_u16(),
                latency: started.elapsed(),
                origin: true,
                reused: false,
                protected: request_decision.protected(),
                bypass: true,
                fingerprint: None,
                shadow_eligible: false,
                score: request_decision.score,
                mode: state.config.mode,
            });
            return status.into_response();
        }
    };

    let status = origin_response.status();
    let origin_headers = clone_response_headers(origin_response.headers());
    let response_bytes = match origin_response.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(error = %err, "origin response body read failed");
            state.observer.record(ObservationRecord {
                route_id,
                cache_key_hash,
                decision: Decision::Bypass,
                reasons: vec![DecisionReason::PolicyError],
                status: StatusCode::BAD_GATEWAY.as_u16(),
                latency: started.elapsed(),
                origin: true,
                reused: false,
                protected: request_decision.protected(),
                bypass: true,
                fingerprint: None,
                shadow_eligible: false,
                score: request_decision.score,
                mode: state.config.mode,
            });
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let response_signals = state.policy.response_signals(status, &origin_headers);
    let fingerprint = make_fingerprint(&state.config, status, &origin_headers, &response_bytes);
    let response_decision = state.policy.decide_response(
        state.config.mode,
        state.observer.route_state(&route_id),
        &request_signals,
        &response_signals,
        response_bytes.len(),
        fingerprint.is_some(),
    );

    let protected = request_decision.decision == Decision::Protect
        || response_decision.decision == Decision::Protect;
    let final_decision = if matches!(
        request_decision.decision,
        Decision::Protect | Decision::Bypass
    ) {
        request_decision.decision
    } else {
        response_decision.decision
    };
    let reasons = if matches!(
        request_decision.decision,
        Decision::Protect | Decision::Bypass
    ) {
        request_decision.reasons.clone()
    } else {
        response_decision.reasons.clone()
    };

    let shadow_eligible = !panic_active
        && state.policy.request_is_reuse_safe(&request_signals)
        && state.policy.response_is_store_safe(&response_signals)
        && fingerprint.is_some()
        && response_bytes.len() as u64 <= state.config.policy.max_fingerprint_body_size;

    state.observer.record(ObservationRecord {
        route_id: route_id.clone(),
        cache_key_hash: cache_key_hash.clone(),
        decision: final_decision,
        reasons: reasons.clone(),
        status: status.as_u16(),
        latency: started.elapsed(),
        origin: true,
        reused: false,
        protected,
        bypass: request_decision.decision == Decision::Bypass,
        fingerprint: fingerprint.clone(),
        shadow_eligible,
        score: response_decision.score,
        mode: state.config.mode,
    });

    if state.config.mode == Mode::Auto
        && !panic_active
        && !protected
        && state.policy.response_is_store_safe(&response_signals)
        && response_bytes.len() as u64 <= state.config.storage.max_object_size
    {
        if let (Some(key_hash), Some(fingerprint)) = (cache_key_hash.clone(), fingerprint) {
            if state.observer.is_auto_eligible(&route_id, &key_hash) {
                let expires_at = SystemTime::now() + state.policy.freshness_ttl();
                let entry = CacheEntry {
                    status: status.as_u16(),
                    headers: sanitized_response_headers(&origin_headers),
                    body: response_bytes.clone(),
                    created_at: SystemTime::now(),
                    expires_at,
                    fingerprint,
                    route_id: route_id.clone(),
                    cache_key_hash: key_hash.clone(),
                };
                if let Err(err) = state.store.put(key_hash.clone(), entry).await {
                    warn!(error = %err, "cache store failed; origin response still returned");
                    state.observer.push_event(
                        EventType::StoreErrorFailOpen,
                        Some(route_id.clone()),
                        Some(key_hash),
                        vec![DecisionReason::StoreError],
                        "cache store failed; returned origin response",
                    );
                }
            }
        }
    }

    response_from_origin(
        &state.config,
        status,
        &origin_headers,
        response_bytes,
        if panic_active {
            "bypass"
        } else if protected {
            "protected"
        } else {
            "miss"
        },
    )
}

async fn send_origin(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<reqwest::Response, reqwest::Error> {
    let url = origin_url(&state.config.origin, uri);
    let req_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = state.client.request(req_method, url);
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        if name == header::HOST
            || is_hop_by_hop_header_named(name.as_str(), &connection_named_headers)
        {
            continue;
        }
        request = request.header(name.as_str(), value.as_bytes());
    }
    request.body(body).send().await
}

fn origin_authority(origin: &Url) -> String {
    let host = origin.host_str().unwrap_or("origin");
    match origin.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

fn origin_url(origin: &Url, uri: &Uri) -> Url {
    let mut url = origin.clone();
    let base_path = origin.path().trim_end_matches('/');
    let request_path = uri.path();
    let path = if base_path.is_empty() || base_path == "/" {
        request_path.to_string()
    } else if request_path == "/" {
        base_path.to_string()
    } else {
        format!("{base_path}{request_path}")
    };
    url.set_path(&path);
    url.set_query(uri.query());
    url
}

fn make_fingerprint(
    config: &EffectiveConfig,
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
) -> Option<ResponseFingerprint> {
    if body.len() as u64 > config.policy.max_fingerprint_body_size {
        return None;
    }
    Some(ResponseFingerprint::new(
        status.as_u16(),
        stable_header_hash(headers),
        Some(body_hash(body)),
    ))
}

fn clone_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut cloned = HeaderMap::new();
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        if !is_hop_by_hop_header_named(name.as_str(), &connection_named_headers) {
            cloned.insert(name.clone(), value.clone());
        }
    }
    cloned
}

fn sanitized_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut sanitized = HeaderMap::new();
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if is_hop_by_hop_header_named(&lower, &connection_named_headers)
            || lower == "set-cookie"
            || lower.starts_with("x-kubio-")
        {
            continue;
        }
        sanitized.insert(name.clone(), value.clone());
    }
    sanitized
}

fn response_from_cache_entry(config: &EffectiveConfig, entry: CacheEntry) -> Response<Body> {
    let mut builder = Response::builder().status(entry.status);
    for (name, value) in &entry.headers {
        if !is_hop_by_hop_header(name.as_str()) {
            builder = builder.header(name, value);
        }
    }
    if config.debug_headers {
        builder = builder.header("x-kubio-status", "hit");
    }
    builder
        .body(Body::from(entry.body))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn response_from_origin(
    config: &EffectiveConfig,
    status: StatusCode,
    headers: &HeaderMap,
    body: Bytes,
    kubio_status: &'static str,
) -> Response<Body> {
    let mut builder = Response::builder().status(status);
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        if !is_hop_by_hop_header_named(name.as_str(), &connection_named_headers) {
            builder = builder.header(name, value);
        }
    }
    if config.debug_headers {
        builder = builder.header("x-kubio-status", kubio_status);
    }
    builder
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn panic_switch_active(path: Option<&Path>) -> bool {
    path.map(|path| path.exists()).unwrap_or(false)
}

fn record_panic_switch_transition(
    state: &ProxyState,
    panic_active: bool,
    route_id: &RouteId,
    cache_key_hash: Option<CacheKeyHash>,
) {
    let was_active = state
        .panic_switch_was_active
        .swap(panic_active, Ordering::Relaxed);

    match (was_active, panic_active) {
        (false, true) => state.observer.push_event(
            EventType::PanicSwitchEnabled,
            Some(route_id.clone()),
            cache_key_hash,
            vec![DecisionReason::PanicSwitchActive],
            "panic switch active; response reuse disabled",
        ),
        (true, false) => state.observer.push_event(
            EventType::PanicSwitchDisabled,
            Some(route_id.clone()),
            cache_key_hash,
            vec![DecisionReason::ReusableAndFresh],
            "panic switch inactive; policy-controlled reuse restored",
        ),
        _ => {}
    }
}

fn connection_header_names(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn is_hop_by_hop_header_named(name: &str, connection_named_headers: &[String]) -> bool {
    is_hop_by_hop_header(name)
        || connection_named_headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case(name))
}

fn count_query_params(query: &str) -> usize {
    if query.is_empty() {
        0
    } else {
        query.split('&').filter(|part| !part.is_empty()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_url_preserves_request_path_and_query() {
        let origin = Url::parse("http://localhost:3000/base").unwrap();
        let uri: Uri = "/api/products?b=2".parse().unwrap();
        assert_eq!(
            origin_url(&origin, &uri).as_str(),
            "http://localhost:3000/base/api/products?b=2"
        );
    }

    #[test]
    fn query_params_are_counted() {
        assert_eq!(count_query_params("a=1&b=2"), 2);
        assert_eq!(count_query_params(""), 0);
    }

    #[test]
    fn connection_named_headers_are_removed_from_origin_responses() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, "x-stream-id".parse().unwrap());
        headers.insert("x-stream-id", "abc".parse().unwrap());
        headers.insert("content-type", "text/plain".parse().unwrap());

        let cloned = clone_response_headers(&headers);

        assert!(!cloned.contains_key(header::CONNECTION));
        assert!(!cloned.contains_key("x-stream-id"));
        assert_eq!(cloned.get("content-type").unwrap(), "text/plain");
    }
}
