//! HTTP reverse proxy runtime for kubio.

use anyhow::Context;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use http::header;
use kubio_core::{
    body_hash, build_cache_key_with_query_config, is_hop_by_hop_header, matching_route_hint,
    query_pattern_matches, stable_header_hash, CacheKeyHash, Decision, DecisionReason,
    EffectiveConfig, Mode, ResponseFingerprint, RouteHintConfig, RouteId, StaleIfErrorMode,
    StoredCacheControl, Validators,
};
use kubio_observe::{
    EventType, ObservationRecord, Observer, QueryParamRecord, RevalidationOutcome,
};
use kubio_policy::PolicyEngine;
use kubio_store::{CacheEntry, CacheStore, PurgeSelector};
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
            .timeout(config.server.origin_timeout)
            .connect_timeout(config.server.origin_timeout.min(Duration::from_secs(5)))
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
    let route_hint = matching_route_hint(&route_id, &state.config.routes);
    let headers = request.headers().clone();
    let panic_active = panic_switch_active(state.config.panic_file.as_deref());
    record_panic_switch_transition(&state, panic_active, &route_id, None);

    let request_body_len = declared_request_body_len(&headers);
    if request_body_len > state.config.policy.max_request_body_size as u64 {
        warn!("request body exceeded proxy body limit");
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let signal_body_len = request_body_len
        .max(unknown_streaming_body_signal(&headers))
        .min(usize::MAX as u64) as usize;

    let mut request_signals =
        state
            .policy
            .request_signals(&method, &path, &headers, signal_body_len);
    if route_hint
        .map(|hint| hint.safety.acknowledge_sensitive_path)
        .unwrap_or(false)
    {
        request_signals.sensitive_path_score = 0;
    }
    request_signals.query_param_count = query
        .as_deref()
        .map(count_query_params)
        .unwrap_or_default()
        .min(u16::MAX as usize) as u16;
    if state.config.policy.query_intelligence.enabled {
        state.observer.record_query_params(
            route_id.clone(),
            query_param_records(query.as_deref(), route_hint),
        );
    }

    let route_state = state.observer.route_state(&route_id);
    let mut request_decision = state.policy.decide_request(
        state.config.mode,
        route_state,
        &request_signals,
        panic_active,
    );
    if route_hint
        .map(|hint| hint.safety.force_protect)
        .unwrap_or(false)
    {
        request_decision = kubio_policy::PolicyDecision::new(
            Decision::Protect,
            vec![DecisionReason::RouteHintApplied],
            kubio_core::RouteState::Protected,
            -100,
        );
    }

    let cache_key_hash = if request_signals.method_cacheable {
        let query_config = route_hint.and_then(|hint| {
            if hint.query.is_empty() {
                None
            } else {
                Some(&hint.query)
            }
        });
        let vary_names = route_hint_vary_names(route_hint);
        Some(
            build_cache_key_with_query_config(
                &method,
                state.config.origin.scheme(),
                &origin_authority(&state.config.origin),
                &path,
                query.as_deref(),
                &headers,
                &vary_names,
                query_config,
            )
            .hash(),
        )
    } else {
        None
    };

    let mut origin_response_override = None;
    let mut stale_error_candidate: Option<(CacheKeyHash, CacheEntry)> = None;

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
                        return response_from_cache_entry_with_status(&state.config, entry, "hit");
                    }
                    Ok(Some(entry)) if entry.is_stale_usable() => {
                        if state.config.policy.revalidation.enabled && entry.validators.available()
                        {
                            match send_conditional_origin(
                                &state,
                                &method,
                                &uri,
                                &headers,
                                &entry.validators,
                            )
                            .await
                            {
                                Ok(response)
                                    if response.status() == reqwest::StatusCode::NOT_MODIFIED =>
                                {
                                    let not_modified_headers =
                                        clone_response_headers(response.headers());
                                    if revalidation_metadata_is_safe(&state, &not_modified_headers)
                                    {
                                        let refreshed = refresh_entry_after_304(
                                            &state,
                                            route_hint,
                                            entry,
                                            &not_modified_headers,
                                        );
                                        if let Err(err) = state
                                            .store
                                            .put(key_hash.clone(), refreshed.clone())
                                            .await
                                        {
                                            warn!(error = %err, "cache refresh failed after 304");
                                        }
                                        state.observer.record_revalidation(
                                            route_id.clone(),
                                            Some(key_hash.clone()),
                                            RevalidationOutcome::NotModified,
                                        );
                                        state.observer.record_reuse(
                                            route_id,
                                            key_hash.clone(),
                                            refreshed.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state.config,
                                            refreshed,
                                            "revalidated",
                                        );
                                    }
                                    if let Err(err) = state
                                        .store
                                        .purge(PurgeSelector::Key(key_hash.clone()))
                                        .await
                                    {
                                        warn!(
                                            error = %err,
                                            "failed to purge cache entry after unsafe 304 metadata"
                                        );
                                        state.observer.push_event(
                                            EventType::StoreErrorFailOpen,
                                            Some(route_id.clone()),
                                            Some(key_hash.clone()),
                                            vec![DecisionReason::StoreError],
                                            "failed to purge cache entry after unsafe 304 metadata",
                                        );
                                    }
                                    origin_response_override = Some(
                                        send_origin(&state, &method, &uri, &headers, Body::empty())
                                            .await
                                            .unwrap_or(response),
                                    );
                                }
                                Ok(response) if response.status().is_server_error() => {
                                    state.observer.record_revalidation(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        RevalidationOutcome::Failed,
                                    );
                                    if stale_if_error_allowed(
                                        &state.config,
                                        route_hint,
                                        &entry,
                                        panic_active,
                                    ) {
                                        state.observer.record_stale(
                                            route_id.clone(),
                                            Some(key_hash.clone()),
                                            true,
                                            DecisionReason::StaleIfErrorAllowed,
                                        );
                                        state.observer.record_reuse(
                                            route_id,
                                            key_hash.clone(),
                                            entry.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state.config,
                                            entry,
                                            "stale",
                                        );
                                    }
                                    state.observer.record_stale(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        false,
                                        stale_denial_reason(&entry),
                                    );
                                    origin_response_override = Some(response);
                                }
                                Ok(response) => {
                                    state.observer.record_revalidation(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        RevalidationOutcome::Modified,
                                    );
                                    origin_response_override = Some(response);
                                }
                                Err(err) => {
                                    warn!(error = %err, "origin revalidation failed");
                                    state.observer.record_revalidation(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        RevalidationOutcome::Failed,
                                    );
                                    if stale_if_error_allowed(
                                        &state.config,
                                        route_hint,
                                        &entry,
                                        panic_active,
                                    ) {
                                        state.observer.record_stale(
                                            route_id.clone(),
                                            Some(key_hash.clone()),
                                            true,
                                            DecisionReason::StaleIfErrorAllowed,
                                        );
                                        state.observer.record_reuse(
                                            route_id,
                                            key_hash.clone(),
                                            entry.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state.config,
                                            entry,
                                            "stale",
                                        );
                                    }
                                    state.observer.record_stale(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        false,
                                        stale_denial_reason(&entry),
                                    );
                                    let status = if err.is_timeout() {
                                        StatusCode::GATEWAY_TIMEOUT
                                    } else {
                                        StatusCode::BAD_GATEWAY
                                    };
                                    return status.into_response();
                                }
                            }
                        } else {
                            state.observer.record_revalidation(
                                route_id.clone(),
                                Some(key_hash.clone()),
                                RevalidationOutcome::Skipped,
                            );
                            stale_error_candidate = Some((key_hash.clone(), entry));
                        }
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

    let origin_response = if let Some(response) = origin_response_override {
        response
    } else {
        match send_origin(&state, &method, &uri, &headers, request.into_body()).await {
            Ok(response) => response,
            Err(err) => {
                warn!(error = %err, "origin request failed");
                let status = if err.is_timeout() {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                };
                if let Some((key_hash, entry)) = stale_error_candidate {
                    if stale_if_error_allowed(&state.config, route_hint, &entry, panic_active) {
                        state.observer.record_stale(
                            route_id.clone(),
                            Some(key_hash.clone()),
                            true,
                            DecisionReason::StaleIfErrorAllowed,
                        );
                        state.observer.record_reuse(
                            route_id,
                            key_hash,
                            entry.status,
                            started.elapsed(),
                        );
                        return response_from_cache_entry_with_status(
                            &state.config,
                            entry,
                            "stale",
                        );
                    }
                    state.observer.record_stale(
                        route_id.clone(),
                        Some(key_hash),
                        false,
                        stale_denial_reason(&entry),
                    );
                }
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
                state.observer.push_event(
                    EventType::OriginRequestFailed,
                    None,
                    None,
                    vec![DecisionReason::PolicyError],
                    if status == StatusCode::GATEWAY_TIMEOUT {
                        "origin request timed out"
                    } else {
                        "origin request failed"
                    },
                );
                return status.into_response();
            }
        }
    };

    let status = origin_response.status();
    let origin_headers = clone_response_headers(origin_response.headers());
    let response_signals = state.policy.response_signals(status, &origin_headers);
    if should_stream_origin_response(
        &state,
        &request_signals,
        &response_signals,
        response_signals.content_length,
    ) {
        let body_len = response_signals
            .content_length
            .unwrap_or(0)
            .min(usize::MAX as u64) as usize;
        let response_decision = state.policy.decide_response(
            state.config.mode,
            state.observer.route_state(&route_id),
            &request_signals,
            &response_signals,
            body_len,
            false,
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

        state.observer.record(ObservationRecord {
            route_id,
            cache_key_hash,
            decision: final_decision,
            reasons,
            status: status.as_u16(),
            latency: started.elapsed(),
            origin: true,
            reused: false,
            protected,
            bypass: request_decision.decision == Decision::Bypass,
            fingerprint: None,
            shadow_eligible: false,
            score: response_decision.score,
            mode: state.config.mode,
        });

        return response_from_origin_stream(
            &state.config,
            status,
            &origin_headers,
            Body::from_stream(origin_response.bytes_stream()),
            if panic_active {
                "bypass"
            } else if protected {
                "protected"
            } else {
                "miss"
            },
        );
    }

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
            state.observer.push_event(
                EventType::OriginRequestFailed,
                None,
                None,
                vec![DecisionReason::PolicyError],
                "origin response body read failed",
            );
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

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
        let validators = state.policy.validators(&origin_headers);
        let cache_control = state.policy.stored_cache_control(&origin_headers);
        let validator_required = cache_control.no_cache || cache_control.must_revalidate;
        if validator_required && !validators.available() {
            state.observer.record_revalidation(
                route_id.clone(),
                cache_key_hash.clone(),
                RevalidationOutcome::Skipped,
            );
        } else if let (Some(key_hash), Some(fingerprint)) = (cache_key_hash.clone(), fingerprint) {
            if state.observer.is_auto_eligible(&route_id, &key_hash) {
                let freshness = entry_freshness(
                    &state,
                    route_hint,
                    &cache_control,
                    &origin_headers,
                    SystemTime::now(),
                );
                let entry = CacheEntry {
                    status: status.as_u16(),
                    headers: sanitized_response_headers(&origin_headers),
                    body: response_bytes.clone(),
                    created_at: freshness.created_at,
                    expires_at: freshness.expires_at,
                    fresh_until: freshness.fresh_until,
                    stale_until: freshness.stale_until,
                    validators,
                    cache_control: cache_control.clone(),
                    must_revalidate: cache_control.no_cache || cache_control.must_revalidate,
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

    response_from_origin_stream(
        &state.config,
        status,
        &origin_headers,
        Body::from(response_bytes),
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
    body: Body,
) -> Result<reqwest::Response, reqwest::Error> {
    send_origin_with_validators(state, method, uri, headers, body, None).await
}

async fn send_conditional_origin(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    validators: &Validators,
) -> Result<reqwest::Response, reqwest::Error> {
    send_origin_with_validators(state, method, uri, headers, Body::empty(), Some(validators)).await
}

async fn send_origin_with_validators(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    validators: Option<&Validators>,
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
    if let Some(validators) = validators {
        if let Some(etag) = validators.etag.as_deref() {
            request = request.header(header::IF_NONE_MATCH.as_str(), etag);
        }
        if let Some(last_modified) = validators.last_modified.as_deref() {
            request = request.header(header::IF_MODIFIED_SINCE.as_str(), last_modified);
        }
    }
    request
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
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

fn response_from_cache_entry_with_status(
    config: &EffectiveConfig,
    entry: CacheEntry,
    kubio_status: &'static str,
) -> Response<Body> {
    let mut builder = Response::builder().status(entry.status);
    for (name, value) in &entry.headers {
        if !is_hop_by_hop_header(name.as_str()) {
            builder = builder.header(name, value);
        }
    }
    if config.debug_headers {
        builder = builder.header("x-kubio-status", kubio_status);
    }
    builder
        .body(Body::from(entry.body))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

#[derive(Debug, Clone)]
struct EntryFreshness {
    created_at: SystemTime,
    fresh_until: SystemTime,
    stale_until: Option<SystemTime>,
    expires_at: SystemTime,
}

fn entry_freshness(
    state: &ProxyState,
    route_hint: Option<&RouteHintConfig>,
    cache_control: &StoredCacheControl,
    headers: &HeaderMap,
    now: SystemTime,
) -> EntryFreshness {
    let base_ttl = state.policy.freshness_ttl_for_route(route_hint);
    let ttl = cache_control
        .max_age
        .map(|max_age| max_age.min(base_ttl))
        .unwrap_or(base_ttl);
    let must_revalidate = cache_control.no_cache || cache_control.must_revalidate;
    let fresh_until = if must_revalidate { now } else { now + ttl };
    let stale_window = stale_window_from_policy(&state.config, route_hint, cache_control, headers);
    let stale_until = stale_window.map(|window| fresh_until + window);
    EntryFreshness {
        created_at: now,
        fresh_until,
        stale_until,
        expires_at: stale_until
            .unwrap_or(fresh_until + state.config.policy.stale_if_error.max_stale),
    }
}

fn stale_window_from_policy(
    config: &EffectiveConfig,
    route_hint: Option<&RouteHintConfig>,
    cache_control: &StoredCacheControl,
    _headers: &HeaderMap,
) -> Option<Duration> {
    let route_window = route_hint.and_then(|hint| {
        if hint.stale_if_error.enabled {
            Some(
                hint.stale_if_error
                    .max_stale
                    .unwrap_or(config.policy.stale_if_error.max_stale),
            )
        } else {
            None
        }
    });
    let origin_window = cache_control.stale_if_error;
    match config.policy.stale_if_error.mode {
        StaleIfErrorMode::Disabled => route_window,
        StaleIfErrorMode::Origin => route_window.or(origin_window),
        StaleIfErrorMode::Enabled => Some(
            route_window
                .or(origin_window)
                .unwrap_or(config.policy.stale_if_error.max_stale),
        ),
    }
    .map(|window| window.min(config.policy.stale_if_error.max_stale))
}

fn stale_if_error_allowed(
    config: &EffectiveConfig,
    route_hint: Option<&RouteHintConfig>,
    entry: &CacheEntry,
    panic_active: bool,
) -> bool {
    !panic_active
        && entry
            .stale_until
            .map(|until| until > SystemTime::now())
            .unwrap_or(false)
        && (entry.cache_control.stale_if_error.is_some()
            || route_hint
                .map(|hint| hint.stale_if_error.enabled)
                .unwrap_or(false)
            || config.policy.stale_if_error.mode == StaleIfErrorMode::Enabled)
}

fn stale_denial_reason(entry: &CacheEntry) -> DecisionReason {
    if entry
        .stale_until
        .map(|until| until <= SystemTime::now())
        .unwrap_or(true)
    {
        DecisionReason::StaleTooOld
    } else {
        DecisionReason::StaleIfErrorNotAllowed
    }
}

fn refresh_entry_after_304(
    state: &ProxyState,
    route_hint: Option<&RouteHintConfig>,
    mut entry: CacheEntry,
    headers: &HeaderMap,
) -> CacheEntry {
    let sanitized = sanitized_response_headers(headers);
    for (name, value) in sanitized {
        if let Some(name) = name {
            entry.headers.insert(name, value);
        }
    }
    let cache_control = state.policy.stored_cache_control(&entry.headers);
    let freshness = entry_freshness(
        state,
        route_hint,
        &cache_control,
        &entry.headers,
        SystemTime::now(),
    );
    entry.created_at = freshness.created_at;
    entry.fresh_until = freshness.fresh_until;
    entry.stale_until = freshness.stale_until;
    entry.expires_at = freshness.expires_at;
    entry.validators = state.policy.validators(&entry.headers);
    entry.cache_control = cache_control.clone();
    entry.must_revalidate = cache_control.no_cache || cache_control.must_revalidate;
    entry
}

fn revalidation_metadata_is_safe(state: &ProxyState, headers: &HeaderMap) -> bool {
    let signals = state.policy.response_signals(StatusCode::OK, headers);
    state.policy.response_hard_deny_reasons(&signals).is_empty()
}

fn route_hint_vary_names(route_hint: Option<&RouteHintConfig>) -> Vec<&str> {
    route_hint
        .filter(|hint| !hint.vary.allow.is_empty())
        .map(|hint| hint.vary.allow.iter().map(String::as_str).collect())
        .unwrap_or_else(|| DEFAULT_VARY_HEADERS.to_vec())
}

fn response_from_origin_stream(
    config: &EffectiveConfig,
    status: StatusCode,
    headers: &HeaderMap,
    body: Body,
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
        .body(body)
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn should_stream_origin_response(
    state: &ProxyState,
    request_signals: &kubio_policy::RequestSignals,
    response_signals: &kubio_policy::ResponseSignals,
    content_length: Option<u64>,
) -> bool {
    !state.policy.request_is_reuse_safe(request_signals)
        || !state.policy.response_is_store_safe(response_signals)
        || content_length
            .map(|length| length > state.config.policy.max_fingerprint_body_size)
            .unwrap_or(false)
}

fn panic_switch_active(path: Option<&Path>) -> bool {
    path.map(|path| path.exists()).unwrap_or(false)
}

fn declared_request_body_len(headers: &HeaderMap) -> u64 {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn unknown_streaming_body_signal(headers: &HeaderMap) -> u64 {
    if headers.contains_key(header::TRANSFER_ENCODING) {
        1
    } else {
        0
    }
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

fn query_param_records(
    query: Option<&str>,
    route_hint: Option<&RouteHintConfig>,
) -> Vec<QueryParamRecord> {
    let Some(query) = query else {
        return Vec::new();
    };
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let name = part.split_once('=').map(|(name, _)| name).unwrap_or(part);
            if name.is_empty() {
                return None;
            }
            Some(QueryParamRecord {
                name: name.to_string(),
                configured_action: query_param_action(name, route_hint).to_string(),
            })
        })
        .collect()
}

fn query_param_action(name: &str, route_hint: Option<&RouteHintConfig>) -> &'static str {
    let Some(hint) = route_hint else {
        return "observe";
    };
    if hint
        .query
        .ignore
        .iter()
        .any(|pattern| query_pattern_matches(pattern, name))
    {
        return "ignore";
    }
    if !hint.query.include.is_empty()
        && !hint
            .query
            .include
            .iter()
            .any(|pattern| query_pattern_matches(pattern, name))
    {
        return "drop";
    }
    "observe"
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
