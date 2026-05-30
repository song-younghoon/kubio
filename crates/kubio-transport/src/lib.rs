//! Network transport boundary for kubio proxy runtimes.

use anyhow::Context;
use axum::body::Body;
use axum::Router;
#[cfg(feature = "experimental-http3")]
use bytes::{Buf, Bytes, BytesMut};
use http::Request;
#[cfg(feature = "experimental-http3")]
use http::Response;
#[cfg(feature = "experimental-http3")]
use http::{HeaderMap, Method, StatusCode};
#[cfg(feature = "experimental-http3")]
use http_body_util::BodyExt;
use hyper::body::Incoming as HyperIncoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder as HyperServerBuilder;
use hyper_util::service::TowerToHyperService;
use kubio_core::{EffectiveConfig, OriginProtocolPreference, TlsConfig};
#[cfg(feature = "experimental-http3")]
use quinn::crypto::rustls::QuicServerConfig as RustlsQuicServerConfig;
#[cfg(feature = "experimental-http3")]
use quinn::{
    Endpoint, EndpointConfig, Incoming as QuinnIncoming, ServerConfig as QuinnServerConfig,
    TokioRuntime, TransportConfig, VarInt,
};
use reqwest::Client;
#[cfg(feature = "experimental-http3")]
use std::collections::HashMap;
#[cfg(feature = "experimental-http3")]
use std::fs::File;
use std::future::Future;
use std::io;
#[cfg(feature = "experimental-http3")]
use std::io::BufReader;
use std::net::SocketAddr;
#[cfg(feature = "experimental-http3")]
use std::pin::Pin;
use std::sync::Arc;
#[cfg(feature = "experimental-http3")]
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
#[cfg(feature = "experimental-http3")]
use tokio::sync::Mutex;
use tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;
use tokio_rustls::TlsAcceptor;
use tower::util::ServiceExt;
use tracing::{debug, warn};

pub const EXPERIMENTAL_HTTP3_FEATURE: &str = "experimental-http3";

pub fn experimental_http3_build_enabled() -> bool {
    cfg!(feature = "experimental-http3")
}

#[cfg(feature = "experimental-http3")]
#[derive(Clone)]
pub struct Http3ServerTelemetry {
    recorder: Arc<dyn Fn(Http3ServerEvent) + Send + Sync>,
}

#[cfg(feature = "experimental-http3")]
impl Http3ServerTelemetry {
    pub fn new<F>(recorder: F) -> Self
    where
        F: Fn(Http3ServerEvent) + Send + Sync + 'static,
    {
        Self {
            recorder: Arc::new(recorder),
        }
    }

    fn record(&self, event: Http3ServerEvent) {
        (self.recorder)(event);
    }
}

#[cfg(feature = "experimental-http3")]
impl Default for Http3ServerTelemetry {
    fn default() -> Self {
        Self::new(|_| {})
    }
}

#[cfg(feature = "experimental-http3")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Http3ServerEvent {
    ConnectionAccepted,
    HandshakeFailed,
    StreamAccepted,
    MalformedRequest,
    RequestBodyRejected,
    ResponseWriteHeadersFailed,
    ResponseWriteBodyFailed,
    ResponseFinishFailed,
}

pub async fn serve_http12_router<F>(
    config: Arc<EffectiveConfig>,
    app: Router,
    shutdown: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind(config.server.listen).await?;
    if let Some(tls) = config.server.tls.as_ref() {
        let acceptor = tls_acceptor(tls, &config)?;
        accept_tls_loop(listener, acceptor, app, config, shutdown).await;
    } else {
        accept_plain_loop(listener, app, config, shutdown).await;
    }
    Ok(())
}

#[cfg(feature = "experimental-http3")]
pub async fn serve_http3_router<F>(
    config: Arc<EffectiveConfig>,
    app: Router,
    telemetry: Http3ServerTelemetry,
    shutdown: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listen = http3_listen_addr(&config);
    let endpoint = http3_endpoint(&config, listen)?;
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                endpoint.close(0_u32.into(), b"shutdown");
                endpoint.wait_idle().await;
                break;
            }
            incoming = endpoint.accept() => {
                match incoming {
                    Some(incoming) => {
                        telemetry.record(Http3ServerEvent::ConnectionAccepted);
                        spawn_h3_connection(
                            incoming,
                            app.clone(),
                            config.clone(),
                            telemetry.clone(),
                        );
                    }
                    None => break,
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "experimental-http3")]
fn http3_listen_addr(config: &EffectiveConfig) -> SocketAddr {
    config.server.http3.listen.unwrap_or(config.server.listen)
}

#[cfg(feature = "experimental-http3")]
#[derive(Clone)]
pub struct Http3OriginClient {
    inner: Arc<Http3OriginClientInner>,
}

#[cfg(feature = "experimental-http3")]
struct Http3OriginClientInner {
    endpoint: Endpoint,
    pool: Mutex<HashMap<String, PooledHttp3Connection>>,
    max_idle_connections: usize,
    origin_timeout: Duration,
}

#[cfg(feature = "experimental-http3")]
struct PooledHttp3Connection {
    send: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    driver: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "experimental-http3")]
#[derive(Debug)]
pub struct Http3OriginResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

#[cfg(feature = "experimental-http3")]
impl Http3OriginResponse {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn into_body(self) -> Bytes {
        self.body
    }
}

#[cfg(feature = "experimental-http3")]
impl Http3OriginClient {
    pub fn new(config: &EffectiveConfig) -> anyhow::Result<Self> {
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
        endpoint.set_default_client_config(http3_origin_client_config(config)?);
        Ok(Self {
            inner: Arc::new(Http3OriginClientInner {
                endpoint,
                pool: Mutex::new(HashMap::new()),
                max_idle_connections: config.origin_protocol.http3_max_idle_connections,
                origin_timeout: config.server.origin_timeout,
            }),
        })
    }

    pub async fn send(
        &self,
        method: &Method,
        url: &url::Url,
        headers: &HeaderMap,
        body: Bytes,
        max_response_body_size: usize,
    ) -> anyhow::Result<Http3OriginResponse> {
        let authority = http3_origin_authority(url)?;
        let host = url
            .host_str()
            .context("HTTP/3 origin URL must include a host")?
            .to_string();
        let timeout = self.inner.origin_timeout;
        tokio::time::timeout(
            timeout,
            self.send_inner(Http3OriginRequest {
                authority,
                host,
                method,
                url,
                headers,
                body,
                max_response_body_size,
            }),
        )
        .await
        .context("HTTP/3 origin request timed out")?
    }

    async fn send_inner(
        &self,
        request: Http3OriginRequest<'_>,
    ) -> anyhow::Result<Http3OriginResponse> {
        let Http3OriginRequest {
            authority,
            host,
            method,
            url,
            headers,
            body,
            max_response_body_size,
        } = request;
        let mut pool = self.inner.pool.lock().await;
        if pool.len() >= self.inner.max_idle_connections && !pool.contains_key(&authority) {
            if let Some(key) = pool.keys().next().cloned() {
                if let Some(pooled) = pool.remove(&key) {
                    pooled.driver.abort();
                }
            }
        }
        if pool
            .get(&authority)
            .map(|pooled| pooled.driver.is_finished())
            .unwrap_or(false)
        {
            pool.remove(&authority);
        }
        if !pool.contains_key(&authority) {
            let pooled = self.connect(&authority, &host).await?;
            pool.insert(authority.clone(), pooled);
        }
        let pooled = pool
            .get_mut(&authority)
            .context("HTTP/3 origin connection was not available")?;
        match send_h3_origin_request(
            &mut pooled.send,
            method,
            url,
            headers,
            body,
            max_response_body_size,
        )
        .await
        {
            Ok(response) => Ok(response),
            Err(err) => {
                pool.remove(&authority);
                Err(err)
            }
        }
    }

    async fn connect(&self, authority: &str, host: &str) -> anyhow::Result<PooledHttp3Connection> {
        let mut addrs = tokio::net::lookup_host(authority)
            .await
            .with_context(|| format!("resolve HTTP/3 origin {authority}"))?;
        let addr = addrs
            .next()
            .with_context(|| format!("HTTP/3 origin {authority} resolved to no addresses"))?;
        let connection = self
            .inner
            .endpoint
            .connect(addr, host)
            .context("connect HTTP/3 origin")?
            .await
            .context("complete HTTP/3 origin QUIC handshake")?;
        let quic = h3_quinn::Connection::new(connection);
        let (mut connection, send) = h3::client::builder()
            .build(quic)
            .await
            .context("start HTTP/3 origin client connection")?;
        let driver = tokio::spawn(async move {
            let _ = connection.wait_idle().await;
        });
        Ok(PooledHttp3Connection { send, driver })
    }
}

#[cfg(feature = "experimental-http3")]
struct Http3OriginRequest<'a> {
    authority: String,
    host: String,
    method: &'a Method,
    url: &'a url::Url,
    headers: &'a HeaderMap,
    body: Bytes,
    max_response_body_size: usize,
}

#[cfg(feature = "experimental-http3")]
fn http3_origin_authority(url: &url::Url) -> anyhow::Result<String> {
    let host = url
        .host_str()
        .context("HTTP/3 origin URL must include a host")?;
    let port = url
        .port_or_known_default()
        .context("HTTP/3 origin URL must include a port or known scheme")?;
    Ok(format!("{host}:{port}"))
}

#[cfg(feature = "experimental-http3")]
async fn send_h3_origin_request(
    send: &mut h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    method: &Method,
    url: &url::Url,
    headers: &HeaderMap,
    body: Bytes,
    max_response_body_size: usize,
) -> anyhow::Result<Http3OriginResponse> {
    let mut builder = Request::builder()
        .method(method.clone())
        .uri(url.as_str())
        .version(http::Version::HTTP_3);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let mut stream = send
        .send_request(builder.body(()).context("build HTTP/3 origin request")?)
        .await
        .context("send HTTP/3 origin request headers")?;
    if !body.is_empty() {
        stream
            .send_data(body)
            .await
            .context("send HTTP/3 origin request body")?;
    }
    stream
        .finish()
        .await
        .context("finish HTTP/3 origin request")?;

    let response = stream
        .recv_response()
        .await
        .context("read HTTP/3 origin response headers")?;
    let status = response.status();
    let headers = response.headers().clone();
    let mut body = BytesMut::new();
    while let Some(mut chunk) = stream
        .recv_data()
        .await
        .context("read HTTP/3 origin response body")?
    {
        let len = chunk.remaining();
        if body.len().saturating_add(len) > max_response_body_size {
            anyhow::bail!("HTTP/3 origin response exceeded configured buffer limit");
        }
        body.extend_from_slice(&chunk.copy_to_bytes(len));
    }
    Ok(Http3OriginResponse {
        status,
        headers,
        body: body.freeze(),
    })
}

#[cfg(feature = "experimental-http3")]
fn http3_origin_client_config(config: &EffectiveConfig) -> anyhow::Result<quinn::ClientConfig> {
    let mut roots = quinn::rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for cert_path in &config.origin_protocol.http3_ca_certs {
        let file = File::open(cert_path)
            .with_context(|| format!("open origin HTTP/3 CA cert {}", cert_path.display()))?;
        for cert in rustls_pemfile::certs(&mut BufReader::new(file)) {
            roots
                .add(cert.context("read origin HTTP/3 CA cert")?)
                .context("add origin HTTP/3 CA cert")?;
        }
    }
    let mut tls = quinn::rustls::ClientConfig::builder_with_provider(Arc::new(
        quinn::rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&quinn::rustls::version::TLS13])
    .context("configure HTTP/3 origin TLS versions")?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let mut client = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .context("build QUIC client TLS config")?,
    ));
    let mut transport = TransportConfig::default();
    transport
        .max_idle_timeout(Some(config.origin_protocol.http3_idle_timeout.try_into()?))
        .datagram_receive_buffer_size(None);
    client.transport_config(Arc::new(transport));
    Ok(client)
}

pub fn origin_client_builder(config: &EffectiveConfig) -> reqwest::ClientBuilder {
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

pub fn origin_uses_http2_prior_knowledge(config: &EffectiveConfig) -> bool {
    config.origin_protocol.http2_prior_knowledge
        || (config.origin_protocol.preferred == OriginProtocolPreference::Http2
            && config.origin.scheme() == "http")
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
    let tower_service = app.map_request(|request: Request<HyperIncoming>| request.map(Body::new));
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
    let mut server = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(load_tls_certificates(tls)?, load_tls_private_key(tls)?)
        .context("build TLS server config")?;
    server.alpn_protocols = alpn_protocols(config);
    Ok(TlsAcceptor::from(Arc::new(server)))
}

#[cfg(feature = "experimental-http3")]
fn http3_endpoint(config: &EffectiveConfig, listen: SocketAddr) -> anyhow::Result<Endpoint> {
    let tls = config
        .server
        .tls
        .as_ref()
        .context("server.http3.enabled requires server.tls")?;
    let mut endpoint_config = EndpointConfig::default();
    endpoint_config
        .max_udp_payload_size(config.server.http3.max_udp_payload_size)
        .context("configure HTTP/3 UDP payload limit")?;

    let server_config = http3_server_config(tls, config)?;
    let socket = std::net::UdpSocket::bind(listen)
        .with_context(|| format!("bind HTTP/3 UDP listener {listen}"))?;
    Endpoint::new(
        endpoint_config,
        Some(server_config),
        socket,
        Arc::new(TokioRuntime),
    )
    .context("create HTTP/3 QUIC endpoint")
}

#[cfg(feature = "experimental-http3")]
fn http3_server_config(
    tls: &TlsConfig,
    config: &EffectiveConfig,
) -> anyhow::Result<QuinnServerConfig> {
    let certs = load_tls_certificates(tls)?;
    let key = load_tls_private_key(tls)?;
    let mut server = RustlsServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&tokio_rustls::rustls::version::TLS13])
    .context("configure HTTP/3 TLS 1.3")?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .context("build HTTP/3 TLS server config")?;
    server.alpn_protocols = vec![b"h3".to_vec()];
    server.max_early_data_size = 0;

    let mut quic = QuinnServerConfig::with_crypto(Arc::new(
        RustlsQuicServerConfig::try_from(server).context("build QUIC TLS server config")?,
    ));
    quic.migration(false);
    quic.transport_config(Arc::new(http3_transport_config(config)?));
    Ok(quic)
}

#[cfg(feature = "experimental-http3")]
fn http3_transport_config(config: &EffectiveConfig) -> anyhow::Result<TransportConfig> {
    let mut transport = TransportConfig::default();
    transport
        .max_concurrent_bidi_streams(varint(config.server.http3.max_concurrent_streams)?)
        .max_concurrent_uni_streams(16_u32.into())
        .max_idle_timeout(Some(config.server.http3.idle_timeout.try_into()?))
        .initial_mtu(config.server.http3.max_udp_payload_size)
        .min_mtu(1200)
        .datagram_receive_buffer_size(None);
    Ok(transport)
}

#[cfg(feature = "experimental-http3")]
fn varint(value: u64) -> anyhow::Result<VarInt> {
    VarInt::from_u64(value).context("HTTP/3 QUIC limit exceeds varint range")
}

fn load_tls_certificates(tls: &TlsConfig) -> anyhow::Result<Vec<CertificateDer<'static>>> {
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
    Ok(certs)
}

fn load_tls_private_key(tls: &TlsConfig) -> anyhow::Result<PrivateKeyDer<'static>> {
    PrivateKeyDer::from_pem_file(&tls.key)
        .with_context(|| format!("read TLS private key {}", tls.key.display()))
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

fn is_connection_accept_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

#[cfg(feature = "experimental-http3")]
fn spawn_h3_connection(
    incoming: QuinnIncoming,
    app: Router,
    config: Arc<EffectiveConfig>,
    telemetry: Http3ServerTelemetry,
) {
    tokio::spawn(async move {
        if let Err(err) = serve_h3_connection(incoming, app, config, telemetry).await {
            debug!(error = %err, "HTTP/3 connection closed with error");
        }
    });
}

#[cfg(feature = "experimental-http3")]
async fn serve_h3_connection(
    incoming: QuinnIncoming,
    app: Router,
    config: Arc<EffectiveConfig>,
    telemetry: Http3ServerTelemetry,
) -> anyhow::Result<()> {
    let connection = match incoming.await {
        Ok(connection) => connection,
        Err(err) => {
            telemetry.record(Http3ServerEvent::HandshakeFailed);
            return Err(err).context("accept HTTP/3 QUIC connection");
        }
    };
    let remote = connection.remote_address();
    let quic = h3_quinn::Connection::new(connection);
    let mut builder = h3::server::builder();
    builder
        .max_field_section_size(config.server.http3.max_field_section_size)
        .send_grease(true);
    let mut connection = builder
        .build(quic)
        .await
        .context("start HTTP/3 server connection")?;

    loop {
        match connection.accept().await {
            Ok(Some(resolver)) => {
                telemetry.record(Http3ServerEvent::StreamAccepted);
                let app = app.clone();
                let config = config.clone();
                let telemetry = telemetry.clone();
                tokio::spawn(async move {
                    if let Err(err) = serve_h3_request(app, config, telemetry, resolver).await {
                        debug!(error = %err, "HTTP/3 request stream failed");
                    }
                });
            }
            Ok(None) => break,
            Err(err) => {
                debug!(remote = %remote, error = %err, "HTTP/3 accept failed");
                break;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "experimental-http3")]
async fn serve_h3_request(
    app: Router,
    config: Arc<EffectiveConfig>,
    telemetry: Http3ServerTelemetry,
    resolver: h3::server::RequestResolver<h3_quinn::Connection, Bytes>,
) -> anyhow::Result<()> {
    let (request, stream) = resolver
        .resolve_request()
        .await
        .inspect_err(|_err| {
            telemetry.record(Http3ServerEvent::MalformedRequest);
        })
        .context("resolve HTTP/3 request")?;
    let (send_stream, recv_stream) = stream.split();
    let body = Body::from_stream(H3RequestBody::new(
        recv_stream,
        config.policy.max_request_body_size,
        telemetry.clone(),
    ));
    let request = h3_request_to_axum(request, body);
    let response = app.oneshot(request).await?;
    write_h3_response(send_stream, response, telemetry).await
}

#[cfg(feature = "experimental-http3")]
struct H3RequestBody<S>
where
    S: h3::quic::RecvStream,
{
    stream: h3::server::RequestStream<S, Bytes>,
    remaining: usize,
    telemetry: Http3ServerTelemetry,
    rejected: bool,
}

#[cfg(feature = "experimental-http3")]
impl<S> H3RequestBody<S>
where
    S: h3::quic::RecvStream,
{
    fn new(
        stream: h3::server::RequestStream<S, Bytes>,
        limit: usize,
        telemetry: Http3ServerTelemetry,
    ) -> Self {
        Self {
            stream,
            remaining: limit,
            telemetry,
            rejected: false,
        }
    }
}

#[cfg(feature = "experimental-http3")]
impl<S> futures_core::Stream for H3RequestBody<S>
where
    S: h3::quic::RecvStream + Unpin,
{
    type Item = Result<Bytes, H3RequestBodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.stream.poll_recv_data(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(Some(mut chunk))) => {
                let len = chunk.remaining();
                if len > this.remaining {
                    if !this.rejected {
                        this.telemetry.record(Http3ServerEvent::RequestBodyRejected);
                        this.rejected = true;
                    }
                    return Poll::Ready(Some(Err(H3RequestBodyError::LimitExceeded)));
                }
                this.remaining -= len;
                Poll::Ready(Some(Ok(chunk.copy_to_bytes(len))))
            }
            Poll::Ready(Ok(None)) => Poll::Ready(None),
            Poll::Ready(Err(_)) => {
                this.telemetry.record(Http3ServerEvent::MalformedRequest);
                Poll::Ready(Some(Err(H3RequestBodyError::Stream)))
            }
        }
    }
}

#[cfg(feature = "experimental-http3")]
#[derive(Debug)]
enum H3RequestBodyError {
    LimitExceeded,
    Stream,
}

#[cfg(feature = "experimental-http3")]
impl std::fmt::Display for H3RequestBodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LimitExceeded => f.write_str("HTTP/3 request body exceeded configured limit"),
            Self::Stream => f.write_str("HTTP/3 request body stream failed"),
        }
    }
}

#[cfg(feature = "experimental-http3")]
impl std::error::Error for H3RequestBodyError {}

#[cfg(feature = "experimental-http3")]
fn h3_request_to_axum(request: Request<()>, body: Body) -> Request<Body> {
    let (mut parts, ()) = request.into_parts();
    parts.version = http::Version::HTTP_3;
    Request::from_parts(parts, body)
}

#[cfg(feature = "experimental-http3")]
async fn write_h3_response<S>(
    mut stream: h3::server::RequestStream<S, Bytes>,
    response: Response<Body>,
    telemetry: Http3ServerTelemetry,
) -> anyhow::Result<()>
where
    S: h3::quic::SendStream<Bytes>,
{
    let (parts, mut body) = response.into_parts();
    let mut builder = Response::builder()
        .status(parts.status)
        .version(http::Version::HTTP_3);
    for (name, value) in parts.headers {
        if let Some(name) = name {
            builder = builder.header(name, value);
        }
    }
    stream
        .send_response(builder.body(()).context("build HTTP/3 response headers")?)
        .await
        .inspect_err(|_err| {
            telemetry.record(Http3ServerEvent::ResponseWriteHeadersFailed);
        })
        .context("send HTTP/3 response headers")?;
    while let Some(frame) = body.frame().await {
        let frame = frame.context("read proxy response body frame")?;
        if let Ok(data) = frame.into_data() {
            if !data.is_empty() {
                stream
                    .send_data(data)
                    .await
                    .inspect_err(|_err| {
                        telemetry.record(Http3ServerEvent::ResponseWriteBodyFailed);
                    })
                    .context("send HTTP/3 response body")?;
            }
        }
    }
    stream
        .finish()
        .await
        .inspect_err(|_err| {
            telemetry.record(Http3ServerEvent::ResponseFinishFailed);
        })
        .context("finish HTTP/3 response")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn origin_prior_knowledge_follows_explicit_flag_or_http_origin_preference() {
        let mut config = EffectiveConfig::default();
        assert!(!origin_uses_http2_prior_knowledge(&config));

        config.origin_protocol.http2_prior_knowledge = true;
        assert!(origin_uses_http2_prior_knowledge(&config));

        config.origin_protocol.http2_prior_knowledge = false;
        config.origin_protocol.preferred = OriginProtocolPreference::Http2;
        config.origin = "https://example.com".parse().unwrap();
        assert!(!origin_uses_http2_prior_knowledge(&config));

        config.origin = "http://example.com".parse().unwrap();
        assert!(origin_uses_http2_prior_knowledge(&config));
    }

    #[cfg(feature = "experimental-http3")]
    #[test]
    fn http3_listen_addr_defaults_to_proxy_listen() {
        let mut config = EffectiveConfig::default();
        config.server.listen = "127.0.0.1:8080".parse().unwrap();
        assert_eq!(http3_listen_addr(&config), config.server.listen);

        config.server.http3.listen = Some("127.0.0.1:8443".parse().unwrap());
        assert_eq!(
            http3_listen_addr(&config),
            "127.0.0.1:8443".parse().unwrap()
        );
    }
}
