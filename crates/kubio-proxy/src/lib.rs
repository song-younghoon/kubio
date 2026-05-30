//! HTTP reverse proxy runtime for kubio.

use anyhow::Context;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use http::header;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder as HyperServerBuilder;
use hyper_util::service::TowerToHyperService;
use kubio_core::{
    body_hash, build_cache_key_with_query_names, is_hop_by_hop_header, is_sensitive_query_param,
    query_pattern_matches, short_hash, stable_header_hash, CacheKeyHash, Decision, DecisionReason,
    EffectiveConfig, HttpProtocol, Mode, OriginProtocolPreference, ResponseFingerprint,
    RouteHintConfig, RouteId, StaleIfErrorMode, StoredCacheControl, TlsConfig, Validators,
};
use kubio_observe::{
    EventType, ObservationRecord, Observer, QueryParamRecord, RevalidationOutcome,
};
use kubio_policy::PolicyEngine;
use kubio_store::{CacheEntry, CacheStore, PurgeSelector};
use reqwest::Client;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;
use tokio_rustls::TlsAcceptor;
use tower::util::ServiceExt;
use tracing::{debug, warn};
use url::{form_urlencoded, Url};

const DEFAULT_VARY_HEADERS: &[&str] = &["accept", "accept-encoding", "accept-language"];

#[derive(Clone)]
pub struct ProxyState {
    pub config: Arc<EffectiveConfig>,
    pub policy: Arc<PolicyEngine>,
    pub observer: Arc<Observer>,
    pub store: Arc<dyn CacheStore>,
    pub client: Client,
    pub fallback_client: Client,
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
        if config.origin_protocol.http2_prior_knowledge
            || (config.origin_protocol.preferred == OriginProtocolPreference::Http2
                && config.origin.scheme() == "http")
        {
            client = client.http2_prior_knowledge();
        }
        let client = client.build().context("build origin HTTP client")?;
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
            route_hints,
            in_flight: Arc::new(Semaphore::new(max_in_flight_requests)),
            panic_switch_was_active: Arc::new(AtomicBool::new(false)),
        })
    }
}

fn origin_client_builder(config: &EffectiveConfig) -> reqwest::ClientBuilder {
    let mut builder = Client::builder()
        .timeout(config.server.origin_timeout)
        .connect_timeout(config.server.origin_timeout.min(Duration::from_secs(5)))
        .pool_max_idle_per_host(config.performance.origin_pool_max_idle_per_host)
        .pool_idle_timeout(config.performance.origin_pool_idle_timeout)
        .http2_initial_stream_window_size(config.server.http2.initial_stream_window_size)
        .http2_initial_connection_window_size(config.server.http2.initial_connection_window_size)
        .http2_max_header_list_size(
            config
                .server
                .http2
                .max_header_list_size
                .min(u64::from(u32::MAX)) as u32,
        )
        .http2_keep_alive_timeout(config.server.http2.keepalive_timeout)
        .http2_keep_alive_while_idle(true);
    if let Some(interval) = config.server.http2.keepalive_interval {
        builder = builder.http2_keep_alive_interval(interval);
    }
    builder
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
    let listener = TcpListener::bind(state.config.server.listen).await?;
    let config = state.config.clone();
    let app = router(state);
    if let Some(tls) = config.server.tls.as_ref() {
        let acceptor = tls_acceptor(tls, &config)?;
        accept_tls_loop(listener, acceptor, app, config, shutdown).await;
    } else {
        accept_plain_loop(listener, app, config, shutdown).await;
    }
    Ok(())
}

async fn accept_plain_loop<F>(
    listener: TcpListener,
    app: Router,
    config: Arc<EffectiveConfig>,
    shutdown: F,
) where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, addr)) => spawn_proxy_connection(stream, addr, app.clone(), config.clone()),
                    Err(err) if is_connection_accept_error(&err) => {}
                    Err(err) => {
                        warn!(error = %err, "proxy accept failed");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

async fn accept_tls_loop<F>(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    app: Router,
    config: Arc<EffectiveConfig>,
    shutdown: F,
) where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, addr)) => {
                        let acceptor = acceptor.clone();
                        let app = app.clone();
                        let config = config.clone();
                        tokio::spawn(async move {
                            match acceptor.accept(stream).await {
                                Ok(tls) => spawn_proxy_connection(tls, addr, app, config),
                                Err(err) => warn!(error = %err, "TLS handshake failed"),
                            }
                        });
                    }
                    Err(err) if is_connection_accept_error(&err) => {}
                    Err(err) => {
                        warn!(error = %err, "proxy accept failed");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

fn spawn_proxy_connection<I>(io: I, addr: SocketAddr, app: Router, config: Arc<EffectiveConfig>)
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(err) = serve_proxy_connection(io, app, &config).await {
            debug!(remote = %addr, error = %err, "proxy connection closed with error");
        }
    });
}

async fn serve_proxy_connection<I>(
    io: I,
    app: Router,
    config: &EffectiveConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let io = TokioIo::new(io);
    let tower_service = app.map_request(|request: Request<Incoming>| request.map(Body::new));
    let hyper_service = TowerToHyperService::new(tower_service);
    let builder = http_server_builder(config);
    builder
        .serve_connection_with_upgrades(io, hyper_service)
        .await
}

fn http_server_builder(config: &EffectiveConfig) -> HyperServerBuilder<TokioExecutor> {
    let mut builder = HyperServerBuilder::new(TokioExecutor::new());
    if config.server.protocols.http1 && !config.server.protocols.http2 {
        builder = builder.http1_only();
    } else if config.server.protocols.http2 && !config.server.protocols.http1 {
        builder = builder.http2_only();
    }

    builder
        .http2()
        .max_concurrent_streams(config.server.http2.max_concurrent_streams)
        .initial_stream_window_size(config.server.http2.initial_stream_window_size)
        .initial_connection_window_size(config.server.http2.initial_connection_window_size)
        .keep_alive_interval(config.server.http2.keepalive_interval)
        .keep_alive_timeout(config.server.http2.keepalive_timeout)
        .max_header_list_size(transport_header_list_limit(
            config.server.http2.max_header_list_size,
        ))
        .timer(TokioTimer::new());
    builder
}

fn transport_header_list_limit(configured: u64) -> u32 {
    configured.saturating_add(1024).min(u64::from(u32::MAX)) as u32
}

fn tls_acceptor(tls: &TlsConfig, config: &EffectiveConfig) -> anyhow::Result<TlsAcceptor> {
    let certs = CertificateDer::pem_file_iter(&tls.cert)
        .with_context(|| format!("open TLS cert {}", tls.cert.display()))?
        .collect::<Result<Vec<_>, _>>()
        .context("read TLS certificates")?;
    if certs.is_empty() {
        anyhow::bail!(
            "TLS cert file {} contained no certificates",
            tls.cert.display()
        );
    }

    let key = PrivateKeyDer::from_pem_file(&tls.key)
        .with_context(|| format!("read TLS private key {}", tls.key.display()))?;

    let mut server = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("build TLS server config")?;
    server.alpn_protocols = alpn_protocols(config);
    Ok(TlsAcceptor::from(Arc::new(server)))
}

fn alpn_protocols(config: &EffectiveConfig) -> Vec<Vec<u8>> {
    let mut protocols = Vec::new();
    if config.server.protocols.http2 {
        protocols.push(b"h2".to_vec());
    }
    if config.server.protocols.http1 {
        protocols.push(b"http/1.1".to_vec());
    }
    protocols
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

fn is_connection_accept_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
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
                                &route_id,
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
    route_id: &RouteId,
) -> Result<reqwest::Response, OriginError> {
    send_origin_with_validators(state, method, uri, headers, body, route_id, None).await
}

async fn send_conditional_origin(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    route_id: &RouteId,
    validators: &Validators,
) -> Result<reqwest::Response, OriginError> {
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
) -> Result<reqwest::Response, OriginError> {
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

async fn send_origin_stream(
    client: &Client,
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    validators: Option<&Validators>,
) -> Result<reqwest::Response, OriginError> {
    let url = origin_url(&state.config.origin, uri);
    let req_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client.request(req_method, url);
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
        .map_err(OriginError::Request)
}

async fn send_origin_bytes(
    client: &Client,
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: bytes::Bytes,
    validators: Option<&Validators>,
) -> Result<reqwest::Response, OriginError> {
    let url = origin_url(&state.config.origin, uri);
    let req_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client.request(req_method, url);
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
        .body(body)
        .send()
        .await
        .map_err(OriginError::Request)
}

fn validate_origin_protocol(
    state: &ProxyState,
    route_id: &RouteId,
    response: reqwest::Response,
) -> Result<reqwest::Response, OriginError> {
    let actual_protocol = http_protocol_from_version(response.version());
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
        && origin_uses_http2_prior_knowledge(state)
        && matches!(method, &Method::GET | &Method::HEAD)
        && declared_request_body_len(headers) == 0
        && !headers.contains_key(header::TRANSFER_ENCODING)
}

fn origin_uses_http2_prior_knowledge(state: &ProxyState) -> bool {
    state.config.origin_protocol.http2_prior_knowledge
        || (state.config.origin_protocol.preferred == OriginProtocolPreference::Http2
            && state.config.origin.scheme() == "http")
}

fn origin_protocol_retry_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_request()
}

#[derive(Debug)]
enum OriginError {
    Request(reqwest::Error),
    BodyRead(String),
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

    #[test]
    fn http_server_builder_respects_enabled_protocols() {
        let mut config = EffectiveConfig::default();
        config.server.protocols.http1 = true;
        config.server.protocols.http2 = false;
        let builder = http_server_builder(&config);
        assert!(builder.is_http1_available());
        assert!(!builder.is_http2_available());

        config.server.protocols.http1 = false;
        config.server.protocols.http2 = true;
        let builder = http_server_builder(&config);
        assert!(!builder.is_http1_available());
        assert!(builder.is_http2_available());
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
