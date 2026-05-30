//! HTTP reverse proxy runtime for kubio.

use anyhow::Context;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use http::header;
use kubio_core::{
    body_hash, build_cache_key_with_query_names, is_hop_by_hop_header, is_sensitive_query_param,
    query_pattern_matches, short_hash, stable_header_hash, CacheKeyHash, Decision, DecisionReason,
    EffectiveConfig, HttpProtocol, Mode, OriginProtocolPreference, ResponseFingerprint,
    RouteHintConfig, RouteId, StaleIfErrorMode, StoredCacheControl, Validators,
};
use kubio_observe::{
    AltSvcOutcome, AltSvcReason, EventType, ObservationRecord, Observer, QueryParamRecord,
    RevalidationOutcome,
};
#[cfg(feature = "experimental-http3")]
use kubio_observe::{Http3ServerEvent as ObservedHttp3ServerEvent, UpstreamHttp3Event};
use kubio_policy::PolicyEngine;
use kubio_store::{CacheEntry, CacheStore, PurgeSelector};
use kubio_transport::{
    origin_client_builder, origin_uses_http2_prior_knowledge, serve_http12_router,
};
#[cfg(feature = "experimental-http3")]
use kubio_transport::{
    serve_http3_router, Http3OriginClient, Http3OriginResponse,
    Http3ServerEvent as TransportHttp3ServerEvent, Http3ServerTelemetry,
};
use reqwest::Client;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
#[cfg(feature = "experimental-http3")]
use tokio::sync::broadcast;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{debug, warn};
use url::{form_urlencoded, Url};

const DEFAULT_VARY_HEADERS: &[&str] = &["accept", "accept-encoding", "accept-language"];
const ALT_SVC_HEADER: &str = "alt-svc";

#[derive(Clone)]
pub struct ProxyState {
    pub config: Arc<EffectiveConfig>,
    pub policy: Arc<PolicyEngine>,
    pub observer: Arc<Observer>,
    pub store: Arc<dyn CacheStore>,
    pub client: Client,
    pub fallback_client: Client,
    #[cfg(feature = "experimental-http3")]
    pub http3_origin_client: Option<Http3OriginClient>,
    route_hints: Arc<RouteHintLookup>,
    in_flight: Arc<Semaphore>,
    panic_switch_was_active: Arc<AtomicBool>,
}

impl ProxyState {
    pub fn new(
        config: Arc<EffectiveConfig>,
        policy: Arc<PolicyEngine>,
        observer: Arc<Observer>,
        store: Arc<dyn CacheStore>,
    ) -> anyhow::Result<Self> {
        let client_builder = origin_client_builder(&config);
        let fallback_client = origin_client_builder(&config)
            .build()
            .context("build fallback origin HTTP client")?;
        let mut client = client_builder;
        if origin_uses_http2_prior_knowledge(&config) {
            client = client.http2_prior_knowledge();
        }
        let client = client.build().context("build origin HTTP client")?;
        #[cfg(feature = "experimental-http3")]
        let http3_origin_client = if config.origin_protocol.http3_experimental {
            Some(Http3OriginClient::new(&config).context("build origin HTTP/3 client")?)
        } else {
            None
        };
        let max_in_flight_requests = config.performance.max_in_flight_requests;
        let route_hints = Arc::new(RouteHintLookup::new(&config.routes));
        observer.record_in_flight(0, max_in_flight_requests);
        Ok(Self {
            config,
            policy,
            observer,
            store,
            client,
            fallback_client,
            #[cfg(feature = "experimental-http3")]
            http3_origin_client,
            route_hints,
            in_flight: Arc::new(Semaphore::new(max_in_flight_requests)),
            panic_switch_was_active: Arc::new(AtomicBool::new(false)),
        })
    }
}

#[derive(Debug)]
struct RouteHintLookup {
    by_route: HashMap<RouteId, PreparedRouteHint>,
    default_vary_names: Vec<String>,
}

impl RouteHintLookup {
    fn new(hints: &[RouteHintConfig]) -> Self {
        let mut by_route = HashMap::with_capacity(hints.len());
        for hint in hints {
            let route_id = RouteId::new(
                hint.route_match.method.to_ascii_uppercase(),
                hint.route_match.path.clone(),
            );
            by_route
                .entry(route_id)
                .or_insert_with(|| PreparedRouteHint {
                    hint: hint.clone(),
                    vary_names: prepared_vary_names(hint),
                });
        }
        Self {
            by_route,
            default_vary_names: DEFAULT_VARY_HEADERS
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        }
    }

    fn get(&self, route_id: &RouteId) -> Option<&PreparedRouteHint> {
        self.by_route.get(route_id)
    }

    fn default_vary_names(&self) -> &[String] {
        &self.default_vary_names
    }
}

#[derive(Debug)]
struct PreparedRouteHint {
    hint: RouteHintConfig,
    vary_names: Vec<String>,
}

fn prepared_vary_names(hint: &RouteHintConfig) -> Vec<String> {
    let names = if hint.vary.allow.is_empty() {
        DEFAULT_VARY_HEADERS
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    } else {
        hint.vary
            .allow
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect()
    };
    names
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
    let config = state.config.clone();
    #[cfg(feature = "experimental-http3")]
    let observer = state.observer.clone();
    let app = router(state);
    if config.server.http3.enabled {
        #[cfg(feature = "experimental-http3")]
        {
            return run_proxy_with_http3(config, app, observer, shutdown).await;
        }
        #[cfg(not(feature = "experimental-http3"))]
        anyhow::bail!(
            "HTTP/3 runtime requires a kubio binary built with --features experimental-http3"
        );
    }
    serve_http12_router(config, app, shutdown).await
}

#[cfg(feature = "experimental-http3")]
async fn run_proxy_with_http3<F>(
    config: Arc<EffectiveConfig>,
    app: Router,
    observer: Arc<Observer>,
    shutdown: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let (shutdown_tx, _) = broadcast::channel::<()>(2);
    let mut tasks = tokio::task::JoinSet::new();
    let mut http12_shutdown = shutdown_tx.subscribe();
    let http12_config = config.clone();
    let http12_app = app.clone();
    tasks.spawn(async move {
        serve_http12_router(http12_config, http12_app, async move {
            let _ = http12_shutdown.recv().await;
        })
        .await
    });

    let mut http3_shutdown = shutdown_tx.subscribe();
    let http3_telemetry = http3_server_telemetry(observer);
    tasks.spawn(async move {
        serve_http3_router(config, app, http3_telemetry, async move {
            let _ = http3_shutdown.recv().await;
        })
        .await
    });

    tokio::select! {
        result = tasks.join_next() => {
            let _ = shutdown_tx.send(());
            if let Some(result) = result {
                result.context("proxy listener task join failed")??;
            }
        }
        _ = shutdown => {
            let _ = shutdown_tx.send(());
            while let Some(result) = tasks.join_next().await {
                result.context("proxy listener task join failed")??;
            }
        }
    }
    while let Some(result) = tasks.join_next().await {
        result.context("proxy listener task join failed")??;
    }
    Ok(())
}

#[cfg(feature = "experimental-http3")]
fn http3_server_telemetry(observer: Arc<Observer>) -> Http3ServerTelemetry {
    Http3ServerTelemetry::new(move |event| {
        observer.record_http3_server_event(match event {
            TransportHttp3ServerEvent::ConnectionAccepted => {
                ObservedHttp3ServerEvent::ConnectionAccepted
            }
            TransportHttp3ServerEvent::HandshakeFailed => ObservedHttp3ServerEvent::HandshakeFailed,
            TransportHttp3ServerEvent::StreamAccepted => ObservedHttp3ServerEvent::StreamAccepted,
            TransportHttp3ServerEvent::MalformedRequest => {
                ObservedHttp3ServerEvent::MalformedRequest
            }
            TransportHttp3ServerEvent::RequestBodyRejected => {
                ObservedHttp3ServerEvent::RequestBodyRejected
            }
            TransportHttp3ServerEvent::ResponseWriteHeadersFailed => {
                ObservedHttp3ServerEvent::ResponseWriteHeadersFailed
            }
            TransportHttp3ServerEvent::ResponseWriteBodyFailed => {
                ObservedHttp3ServerEvent::ResponseWriteBodyFailed
            }
            TransportHttp3ServerEvent::ResponseFinishFailed => {
                ObservedHttp3ServerEvent::ResponseFinishFailed
            }
        });
    })
}

struct ObservedInFlightPermit {
    permit: Option<OwnedSemaphorePermit>,
    semaphore: Arc<Semaphore>,
    observer: Arc<Observer>,
    max: usize,
}

impl ObservedInFlightPermit {
    fn new(state: &ProxyState, permit: OwnedSemaphorePermit) -> Self {
        let current = state
            .config
            .performance
            .max_in_flight_requests
            .saturating_sub(state.in_flight.available_permits());
        state
            .observer
            .record_in_flight(current, state.config.performance.max_in_flight_requests);
        Self {
            permit: Some(permit),
            semaphore: state.in_flight.clone(),
            observer: state.observer.clone(),
            max: state.config.performance.max_in_flight_requests,
        }
    }
}

impl Drop for ObservedInFlightPermit {
    fn drop(&mut self) {
        drop(self.permit.take());
        let current = self.max.saturating_sub(self.semaphore.available_permits());
        self.observer.record_in_flight(current, self.max);
    }
}

async fn proxy_handler(State(state): State<ProxyState>, request: Request<Body>) -> Response<Body> {
    let started = std::time::Instant::now();
    let downstream_protocol = http_protocol_from_version(request.version());
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(ToOwned::to_owned);
    let route_id = RouteId::from_method_path(&method, &path);
    let headers = request.headers().clone();
    let request_authority = request_authority(&uri, &headers);
    if downstream_protocol == HttpProtocol::Http2
        && header_list_size(&headers) > state.config.server.http2.max_header_list_size
    {
        state
            .observer
            .record_header_limit_rejection(route_id, downstream_protocol);
        return StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE.into_response();
    }
    let Ok(permit) = state.in_flight.clone().try_acquire_owned() else {
        state
            .observer
            .record_backpressure_rejection(route_id, downstream_protocol);
        state.observer.record_in_flight(
            state
                .config
                .performance
                .max_in_flight_requests
                .saturating_sub(state.in_flight.available_permits()),
            state.config.performance.max_in_flight_requests,
        );
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let _permit = ObservedInFlightPermit::new(&state, permit);
    state
        .observer
        .record_downstream_protocol(route_id.clone(), downstream_protocol);
    let route_hint_entry = state.route_hints.get(&route_id);
    let route_hint = route_hint_entry.map(|entry| &entry.hint);
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
    let query_records = if state.config.policy.query_intelligence.enabled {
        query_param_records(query.as_deref(), route_hint)
    } else {
        Vec::new()
    };
    state
        .observer
        .record_query_params(route_id.clone(), query_records.clone());

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
    record_hint_observations(
        &state,
        &route_id,
        route_hint,
        &request_signals,
        &request_decision,
    );

    let cache_key_hash = if request_signals.method_cacheable {
        let query_config = route_hint.and_then(|hint| {
            if hint.query.is_empty() {
                None
            } else {
                Some(&hint.query)
            }
        });
        let vary_names = route_hint_entry
            .map(|entry| entry.vary_names.as_slice())
            .unwrap_or_else(|| state.route_hints.default_vary_names());
        Some(
            build_cache_key_with_query_names(
                &method,
                state.config.origin.scheme(),
                &origin_authority(&state.config.origin),
                &path,
                query.as_deref(),
                &headers,
                vary_names.iter().map(String::as_str),
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
                            route_id.clone(),
                            key_hash.clone(),
                            entry.status,
                            started.elapsed(),
                        );
                        return response_from_cache_entry_with_status(
                            &state,
                            &route_id,
                            entry,
                            "hit",
                            request_authority.as_deref(),
                        );
                    }
                    Ok(Some(entry)) if entry.is_stale_usable() => {
                        if state.config.policy.revalidation.enabled && entry.validators.available()
                        {
                            match send_conditional_origin(
                                &state,
                                &method,
                                &uri,
                                &headers,
                                &route_id,
                                &entry.validators,
                            )
                            .await
                            {
                                Ok(response) if response.status() == StatusCode::NOT_MODIFIED => {
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
                                            route_id.clone(),
                                            key_hash.clone(),
                                            refreshed.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state,
                                            &route_id,
                                            refreshed,
                                            "revalidated",
                                            request_authority.as_deref(),
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
                                        send_origin(
                                            &state,
                                            &method,
                                            &uri,
                                            &headers,
                                            Body::empty(),
                                            &route_id,
                                        )
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
                                            route_id.clone(),
                                            key_hash.clone(),
                                            entry.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state,
                                            &route_id,
                                            entry,
                                            "stale",
                                            request_authority.as_deref(),
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
                                            route_id.clone(),
                                            key_hash.clone(),
                                            entry.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state,
                                            &route_id,
                                            entry,
                                            "stale",
                                            request_authority.as_deref(),
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
        match send_origin(
            &state,
            &method,
            &uri,
            &headers,
            request.into_body(),
            &route_id,
        )
        .await
        {
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
                            route_id.clone(),
                            key_hash,
                            entry.status,
                            started.elapsed(),
                        );
                        return response_from_cache_entry_with_status(
                            &state,
                            &route_id,
                            entry,
                            "stale",
                            request_authority.as_deref(),
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
    record_store_saturation_if_needed(
        &state,
        &route_id,
        cache_key_hash.as_ref(),
        &request_signals,
        &response_signals,
        response_signals.content_length,
    );
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
            route_id: route_id.clone(),
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
            &state,
            &route_id,
            status,
            &origin_headers,
            origin_response.into_body_stream(),
            request_authority.as_deref(),
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
    record_store_saturation_if_needed(
        &state,
        &route_id,
        cache_key_hash.as_ref(),
        &request_signals,
        &response_signals,
        Some(response_bytes.len() as u64),
    );

    let fingerprint = make_fingerprint(&state.config, status, &origin_headers, &response_bytes);
    if let Some(fingerprint) = fingerprint.as_ref() {
        state
            .observer
            .record_query_fingerprint(route_id.clone(), &query_records, fingerprint);
    }
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
        &state,
        &route_id,
        status,
        &origin_headers,
        Body::from(response_bytes),
        request_authority.as_deref(),
        if panic_active {
            "bypass"
        } else if protected {
            "protected"
        } else {
            "miss"
        },
    )
}

enum OriginResponse {
    Reqwest(reqwest::Response),
    #[cfg(feature = "experimental-http3")]
    Http3(Http3OriginResponse),
}

impl OriginResponse {
    fn status(&self) -> StatusCode {
        match self {
            Self::Reqwest(response) => response.status(),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(response) => response.status(),
        }
    }

    fn headers(&self) -> &HeaderMap {
        match self {
            Self::Reqwest(response) => response.headers(),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(response) => response.headers(),
        }
    }

    fn protocol(&self) -> HttpProtocol {
        match self {
            Self::Reqwest(response) => http_protocol_from_version(response.version()),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(_) => HttpProtocol::Http3,
        }
    }

    async fn bytes(self) -> Result<bytes::Bytes, OriginError> {
        match self {
            Self::Reqwest(response) => response.bytes().await.map_err(OriginError::Request),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(response) => Ok(response.into_body()),
        }
    }

    fn into_body_stream(self) -> Body {
        match self {
            Self::Reqwest(response) => Body::from_stream(response.bytes_stream()),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(response) => Body::from(response.into_body()),
        }
    }
}

async fn send_origin(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    route_id: &RouteId,
) -> Result<OriginResponse, OriginError> {
    send_origin_with_validators(state, method, uri, headers, body, route_id, None).await
}

async fn send_conditional_origin(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    route_id: &RouteId,
    validators: &Validators,
) -> Result<OriginResponse, OriginError> {
    send_origin_with_validators(
        state,
        method,
        uri,
        headers,
        Body::empty(),
        route_id,
        Some(validators),
    )
    .await
}

async fn send_origin_with_validators(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    route_id: &RouteId,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    #[cfg(feature = "experimental-http3")]
    if origin_http3_attempt_enabled(state) {
        return send_origin_http3_with_fallback(
            state, method, uri, headers, body, route_id, validators,
        )
        .await;
    }

    if origin_protocol_retry_is_possible(state, method, headers) {
        let body = axum::body::to_bytes(body, state.config.policy.max_request_body_size)
            .await
            .map_err(|err| OriginError::BodyRead(err.to_string()))?;
        match send_origin_bytes(
            &state.client,
            state,
            method,
            uri,
            headers,
            body.clone(),
            validators,
        )
        .await
        {
            Ok(response) => return validate_origin_protocol(state, route_id, response),
            Err(OriginError::Request(err)) if origin_protocol_retry_error(&err) => {
                let response = send_origin_bytes(
                    &state.fallback_client,
                    state,
                    method,
                    uri,
                    headers,
                    body,
                    validators,
                )
                .await?;
                return validate_origin_protocol(state, route_id, response);
            }
            Err(err) => return Err(err),
        }
    }

    let response =
        send_origin_stream(&state.client, state, method, uri, headers, body, validators).await?;
    validate_origin_protocol(state, route_id, response)
}

#[cfg(feature = "experimental-http3")]
async fn send_origin_http3_with_fallback(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    route_id: &RouteId,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    if state.config.origin.scheme() != "https" {
        state
            .observer
            .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::SkippedNotHttps);
        return send_origin_after_http3_skip(
            state, method, uri, headers, body, route_id, validators,
        )
        .await;
    }

    let replayable = request_is_replayable_for_protocol_fallback(method, headers);
    if state.config.origin_protocol.fallback && !replayable {
        state.observer.record_upstream_http3_event(
            route_id.clone(),
            UpstreamHttp3Event::SkippedNonReplayable,
        );
        return Err(OriginError::NonReplayableHttp3FallbackBlocked);
    }

    let body = axum::body::to_bytes(body, state.config.policy.max_request_body_size)
        .await
        .map_err(|err| OriginError::BodyRead(err.to_string()))?;
    state
        .observer
        .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Attempt);
    match send_origin_http3_bytes(state, method, uri, headers, body.clone(), validators).await {
        Ok(response) => {
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Success);
            validate_origin_protocol(state, route_id, OriginResponse::Http3(response))
        }
        Err(err) if state.config.origin_protocol.fallback && replayable => {
            warn!(error = %err, "upstream HTTP/3 attempt failed; falling back");
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Failure);
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Fallback);
            let response = send_origin_bytes(
                &state.fallback_client,
                state,
                method,
                uri,
                headers,
                body,
                validators,
            )
            .await?;
            validate_origin_protocol(state, route_id, response)
        }
        Err(err) => {
            warn!(error = %err, "required upstream HTTP/3 attempt failed");
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Failure);
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::RequiredFailure);
            Err(OriginError::Http3RequiredFailed(err.to_string()))
        }
    }
}

#[cfg(feature = "experimental-http3")]
async fn send_origin_after_http3_skip(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    route_id: &RouteId,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    if state.config.origin_protocol.fallback
        && request_is_replayable_for_protocol_fallback(method, headers)
    {
        let body = axum::body::to_bytes(body, state.config.policy.max_request_body_size)
            .await
            .map_err(|err| OriginError::BodyRead(err.to_string()))?;
        let response = send_origin_bytes(
            &state.fallback_client,
            state,
            method,
            uri,
            headers,
            body,
            validators,
        )
        .await?;
        state
            .observer
            .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Fallback);
        validate_origin_protocol(state, route_id, response)
    } else {
        if state.config.origin_protocol.fallback {
            state.observer.record_upstream_http3_event(
                route_id.clone(),
                UpstreamHttp3Event::SkippedNonReplayable,
            );
            return Err(OriginError::NonReplayableHttp3FallbackBlocked);
        }
        state
            .observer
            .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::RequiredFailure);
        Err(OriginError::Http3RequiredFailed(
            "origin is not HTTPS".to_string(),
        ))
    }
}

#[cfg(feature = "experimental-http3")]
async fn send_origin_http3_bytes(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: bytes::Bytes,
    validators: Option<&Validators>,
) -> anyhow::Result<Http3OriginResponse> {
    let client = state
        .http3_origin_client
        .as_ref()
        .context("origin HTTP/3 client is not configured")?;
    let url = origin_url(&state.config.origin, uri);
    let headers = origin_request_headers(headers, validators);
    let max_response_body_size = state
        .config
        .performance
        .max_buffered_response_size
        .max(state.config.storage.max_object_size)
        .max(state.config.policy.max_fingerprint_body_size)
        .min(usize::MAX as u64) as usize;
    client
        .send(method, &url, &headers, body, max_response_body_size)
        .await
}

#[cfg(feature = "experimental-http3")]
fn origin_http3_attempt_enabled(state: &ProxyState) -> bool {
    state.config.origin_protocol.http3_experimental
        && state.config.origin_protocol.preferred == OriginProtocolPreference::Http3
}

async fn send_origin_stream(
    client: &Client,
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    let url = origin_url(&state.config.origin, uri);
    let req_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client.request(req_method, url);
    let origin_headers = origin_request_headers(headers, validators);
    for (name, value) in &origin_headers {
        request = request.header(name.as_str(), value.as_bytes());
    }
    request
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
        .map_err(OriginError::Request)
        .map(OriginResponse::Reqwest)
}

async fn send_origin_bytes(
    client: &Client,
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: bytes::Bytes,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    let url = origin_url(&state.config.origin, uri);
    let req_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client.request(req_method, url);
    let origin_headers = origin_request_headers(headers, validators);
    for (name, value) in &origin_headers {
        request = request.header(name.as_str(), value.as_bytes());
    }
    request
        .body(body)
        .send()
        .await
        .map_err(OriginError::Request)
        .map(OriginResponse::Reqwest)
}

fn origin_request_headers(headers: &HeaderMap, validators: Option<&Validators>) -> HeaderMap {
    let mut origin_headers = HeaderMap::new();
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        if name == header::HOST
            || is_hop_by_hop_header_named(name.as_str(), &connection_named_headers)
        {
            continue;
        }
        origin_headers.insert(name.clone(), value.clone());
    }
    if let Some(validators) = validators {
        if let Some(etag) = validators.etag.as_deref() {
            if let Ok(value) = HeaderValue::from_str(etag) {
                origin_headers.insert(header::IF_NONE_MATCH, value);
            }
        }
        if let Some(last_modified) = validators.last_modified.as_deref() {
            if let Ok(value) = HeaderValue::from_str(last_modified) {
                origin_headers.insert(header::IF_MODIFIED_SINCE, value);
            }
        }
    }
    origin_headers
}

fn validate_origin_protocol(
    state: &ProxyState,
    route_id: &RouteId,
    response: OriginResponse,
) -> Result<OriginResponse, OriginError> {
    let actual_protocol = response.protocol();
    state
        .observer
        .record_upstream_protocol(route_id.clone(), actual_protocol);
    if let Some(expected_protocol) =
        expected_origin_protocol(state.config.origin_protocol.preferred)
    {
        if actual_protocol != expected_protocol {
            if state.config.origin_protocol.fallback {
                state.observer.record_protocol_fallback(
                    route_id.clone(),
                    expected_protocol,
                    actual_protocol,
                );
            } else {
                return Err(OriginError::RequiredProtocol {
                    expected: expected_protocol,
                    actual: actual_protocol,
                });
            }
        }
    }
    Ok(response)
}

fn origin_protocol_retry_is_possible(
    state: &ProxyState,
    method: &Method,
    headers: &HeaderMap,
) -> bool {
    state.config.origin_protocol.fallback
        && origin_uses_http2_prior_knowledge(&state.config)
        && request_is_replayable_for_protocol_fallback(method, headers)
}

fn request_is_replayable_for_protocol_fallback(method: &Method, headers: &HeaderMap) -> bool {
    matches!(method, &Method::GET | &Method::HEAD)
        && declared_request_body_len(headers) == 0
        && !headers.contains_key(header::TRANSFER_ENCODING)
}

fn origin_protocol_retry_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_request()
}

#[derive(Debug)]
enum OriginError {
    Request(reqwest::Error),
    BodyRead(String),
    #[cfg(feature = "experimental-http3")]
    Http3RequiredFailed(String),
    #[cfg(feature = "experimental-http3")]
    NonReplayableHttp3FallbackBlocked,
    RequiredProtocol {
        expected: HttpProtocol,
        actual: HttpProtocol,
    },
}

impl OriginError {
    fn is_timeout(&self) -> bool {
        matches!(self, Self::Request(err) if err.is_timeout())
    }
}

impl fmt::Display for OriginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(err) => err.fmt(f),
            Self::BodyRead(err) => write!(f, "origin request body read failed: {err}"),
            #[cfg(feature = "experimental-http3")]
            Self::Http3RequiredFailed(err) => {
                write!(f, "required upstream HTTP/3 failed: {err}")
            }
            #[cfg(feature = "experimental-http3")]
            Self::NonReplayableHttp3FallbackBlocked => {
                f.write_str("upstream HTTP/3 fallback blocked for non-replayable request")
            }
            Self::RequiredProtocol { expected, actual } => {
                write!(
                    f,
                    "origin used {actual} when {expected} was required by origin_protocol"
                )
            }
        }
    }
}

impl std::error::Error for OriginError {}

fn expected_origin_protocol(preferred: OriginProtocolPreference) -> Option<HttpProtocol> {
    match preferred {
        OriginProtocolPreference::Auto => None,
        OriginProtocolPreference::Http1 => Some(HttpProtocol::Http1),
        OriginProtocolPreference::Http2 => Some(HttpProtocol::Http2),
        OriginProtocolPreference::Http3 => Some(HttpProtocol::Http3),
    }
}

fn http_protocol_from_version(version: http::Version) -> HttpProtocol {
    match version {
        http::Version::HTTP_2 => HttpProtocol::Http2,
        http::Version::HTTP_3 => HttpProtocol::Http3,
        _ => HttpProtocol::Http1,
    }
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
    state: &ProxyState,
    route_id: &RouteId,
    entry: CacheEntry,
    kubio_status: &'static str,
    request_authority: Option<&str>,
) -> Response<Body> {
    let mut builder = Response::builder().status(entry.status);
    for (name, value) in &entry.headers {
        if !is_hop_by_hop_header(name.as_str()) && name.as_str() != ALT_SVC_HEADER {
            builder = builder.header(name, value);
        }
    }
    if state.config.debug_headers {
        builder = builder.header("x-kubio-status", kubio_status);
    }
    builder = add_alt_svc_header(builder, state, route_id, request_authority);
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
            if name == header::CONTENT_LENGTH || name == header::TRANSFER_ENCODING {
                continue;
            }
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

fn record_hint_observations(
    state: &ProxyState,
    route_id: &RouteId,
    route_hint: Option<&RouteHintConfig>,
    request_signals: &kubio_policy::RequestSignals,
    request_decision: &kubio_policy::PolicyDecision,
) {
    let Some(hint) = route_hint else {
        return;
    };

    let rejected_by_hard_deny = request_decision.decision == Decision::Protect
        && !request_decision
            .reasons
            .contains(&DecisionReason::RouteHintApplied);
    state.observer.record_route_hint(
        route_id.clone(),
        hint.display_name(),
        !rejected_by_hard_deny,
        if rejected_by_hard_deny {
            DecisionReason::RouteHintRejected
        } else {
            DecisionReason::RouteHintApplied
        },
    );

    if !hint.query.is_empty() {
        let query_hint_applied = request_signals.method_cacheable && !rejected_by_hard_deny;
        state.observer.record_query_hint(
            route_id.clone(),
            query_hint_applied,
            if query_hint_applied {
                DecisionReason::QueryHintApplied
            } else {
                DecisionReason::QueryHintRejected
            },
        );
    }
}

fn response_from_origin_stream(
    state: &ProxyState,
    route_id: &RouteId,
    status: StatusCode,
    headers: &HeaderMap,
    body: Body,
    request_authority: Option<&str>,
    kubio_status: &'static str,
) -> Response<Body> {
    let mut builder = Response::builder().status(status);
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        if !is_hop_by_hop_header_named(name.as_str(), &connection_named_headers)
            && name.as_str() != ALT_SVC_HEADER
        {
            builder = builder.header(name, value);
        }
    }
    if state.config.debug_headers {
        builder = builder.header("x-kubio-status", kubio_status);
    }
    builder = add_alt_svc_header(builder, state, route_id, request_authority);
    builder
        .body(body)
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn request_authority(uri: &Uri, headers: &HeaderMap) -> Option<String> {
    uri.authority()
        .map(|authority| authority.as_str().to_string())
        .or_else(|| {
            headers
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned)
        })
}

fn add_alt_svc_header(
    mut builder: http::response::Builder,
    state: &ProxyState,
    route_id: &RouteId,
    request_authority: Option<&str>,
) -> http::response::Builder {
    let decision = alt_svc_decision(&state.config, request_authority);
    state
        .observer
        .record_alt_svc(route_id.clone(), decision.outcome, decision.reason);
    if let Some(value) = decision.value {
        builder = builder.header(ALT_SVC_HEADER, value);
    }
    builder
}

#[derive(Debug)]
struct AltSvcDecision {
    outcome: AltSvcOutcome,
    reason: AltSvcReason,
    value: Option<HeaderValue>,
}

fn alt_svc_decision(config: &EffectiveConfig, request_authority: Option<&str>) -> AltSvcDecision {
    if !config.server.http3.enabled {
        return AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::Http3Disabled,
            value: None,
        };
    }
    if !config.server.http3.advertise {
        return AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::AdvertiseDisabled,
            value: None,
        };
    }
    let Some(request_authority) = request_authority else {
        return AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::MissingAuthority,
            value: None,
        };
    };
    if !config
        .server
        .http3
        .authorities
        .iter()
        .any(|authority| authority.eq_ignore_ascii_case(request_authority))
    {
        return AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::AuthorityNotAllowed,
            value: None,
        };
    }

    let listen = config.server.http3.listen.unwrap_or(config.server.listen);
    let value = format!(
        "h3=\":{}\"; ma={}",
        listen.port(),
        config.server.http3.alt_svc_ma.as_secs()
    );
    match HeaderValue::from_str(&value) {
        Ok(value) => AltSvcDecision {
            outcome: AltSvcOutcome::Advertised,
            reason: AltSvcReason::ConfiguredAuthority,
            value: Some(value),
        },
        Err(_) => AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::InvalidValue,
            value: None,
        },
    }
}

fn should_stream_origin_response(
    state: &ProxyState,
    request_signals: &kubio_policy::RequestSignals,
    response_signals: &kubio_policy::ResponseSignals,
    content_length: Option<u64>,
) -> bool {
    let known_too_large = content_length
        .map(|length| {
            length > state.config.policy.max_fingerprint_body_size
                || length > state.config.storage.max_object_size
                || length > state.config.performance.max_buffered_response_size
        })
        .unwrap_or(false);
    (state.config.performance.stream_unstoreable_bodies
        && (!state.policy.request_is_reuse_safe(request_signals)
            || !state.policy.response_is_store_safe(response_signals)))
        || known_too_large
}

fn record_store_saturation_if_needed(
    state: &ProxyState,
    route_id: &RouteId,
    cache_key_hash: Option<&CacheKeyHash>,
    request_signals: &kubio_policy::RequestSignals,
    response_signals: &kubio_policy::ResponseSignals,
    response_size: Option<u64>,
) {
    let Some(response_size) = response_size else {
        return;
    };
    if response_size <= state.config.storage.max_object_size {
        return;
    }
    if !state.policy.request_is_reuse_safe(request_signals)
        || !state.policy.response_is_store_safe(response_signals)
    {
        return;
    }
    state.observer.push_event(
        EventType::StoreSaturated,
        Some(route_id.clone()),
        cache_key_hash.cloned(),
        vec![DecisionReason::ObjectTooLarge],
        "response was larger than the configured store object limit",
    );
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

fn header_list_size(headers: &HeaderMap) -> u64 {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() as u64 + value.as_bytes().len() as u64)
        .sum()
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
    form_urlencoded::parse(query.as_bytes())
        .filter_map(|(name, value)| {
            if name.is_empty() {
                return None;
            }
            let sensitive = is_sensitive_query_param(&name);
            let value_hash = if sensitive {
                None
            } else {
                Some(short_hash(&format!("{name}={value}")))
            };
            Some(QueryParamRecord {
                configured_action: query_param_action(&name, route_hint).to_string(),
                name: name.into_owned(),
                value_hash,
                sensitive,
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

    #[test]
    fn route_hint_lookup_matches_case_insensitively_and_keeps_first_hint() {
        let first = route_hint("get", "/api/products", Some("first"), &["accept-language"]);
        let duplicate = route_hint("GET", "/api/products", Some("second"), &["x-variant"]);
        let lookup = RouteHintLookup::new(&[first, duplicate]);

        let prepared = lookup
            .get(&RouteId::new("GET", "/api/products"))
            .expect("route hint should be indexed");

        assert_eq!(prepared.hint.display_name(), "first");
        assert_eq!(prepared.vary_names, vec!["accept-language"]);
        assert!(lookup.get(&RouteId::new("POST", "/api/products")).is_none());
    }

    fn route_hint(method: &str, path: &str, name: Option<&str>, vary: &[&str]) -> RouteHintConfig {
        RouteHintConfig {
            name: name.map(ToOwned::to_owned),
            route_match: kubio_core::RouteMatchConfig {
                method: method.to_string(),
                path: path.to_string(),
            },
            freshness: Default::default(),
            query: Default::default(),
            vary: kubio_core::RouteVaryConfig {
                allow: vary.iter().map(|name| (*name).to_string()).collect(),
            },
            stale_if_error: Default::default(),
            safety: Default::default(),
        }
    }
}
