use anyhow::Context;
use axum::body::Body;
use axum::Router;
use bytes::{Buf, Bytes, BytesMut};
use http::{HeaderMap, Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use kubio_core::{EffectiveConfig, TlsConfig};
use quinn::crypto::rustls::QuicServerConfig as RustlsQuicServerConfig;
use quinn::rustls::pki_types::{pem::PemObject, CertificateDer};
use quinn::{
    Endpoint, EndpointConfig, Incoming as QuinnIncoming, ServerConfig as QuinnServerConfig,
    TokioRuntime, TransportConfig, VarInt,
};
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;
use tower::util::ServiceExt;
use tracing::debug;

use crate::tls::{load_tls_certificates, load_tls_private_key};

#[derive(Clone)]
pub struct Http3ServerTelemetry {
    recorder: Arc<dyn Fn(Http3ServerEvent) + Send + Sync>,
}

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

impl Default for Http3ServerTelemetry {
    fn default() -> Self {
        Self::new(|_| {})
    }
}

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

fn http3_listen_addr(config: &EffectiveConfig) -> SocketAddr {
    config.server.http3.listen.unwrap_or(config.server.listen)
}

#[derive(Clone)]
pub struct Http3OriginClient {
    inner: Arc<Http3OriginClientInner>,
}

struct Http3OriginClientInner {
    endpoint: Endpoint,
    pool: Mutex<HashMap<String, PooledHttp3Connection>>,
    max_idle_connections: usize,
    origin_timeout: Duration,
}

struct PooledHttp3Connection {
    send: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    driver: tokio::task::JoinHandle<()>,
}

#[derive(Debug)]
pub struct Http3OriginResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

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

struct Http3OriginRequest<'a> {
    authority: String,
    host: String,
    method: &'a Method,
    url: &'a url::Url,
    headers: &'a HeaderMap,
    body: Bytes,
    max_response_body_size: usize,
}

fn http3_origin_authority(url: &url::Url) -> anyhow::Result<String> {
    let host = url
        .host_str()
        .context("HTTP/3 origin URL must include a host")?;
    let port = url
        .port_or_known_default()
        .context("HTTP/3 origin URL must include a port or known scheme")?;
    Ok(format!("{host}:{port}"))
}

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

fn http3_origin_client_config(config: &EffectiveConfig) -> anyhow::Result<quinn::ClientConfig> {
    let mut roots = quinn::rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for cert_path in &config.origin_protocol.http3_ca_certs {
        let certs = CertificateDer::pem_file_iter(cert_path)
            .with_context(|| format!("open origin HTTP/3 CA cert {}", cert_path.display()))?;
        for cert in certs {
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

fn varint(value: u64) -> anyhow::Result<VarInt> {
    VarInt::from_u64(value).context("HTTP/3 QUIC limit exceeds varint range")
}

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

struct H3RequestBody<S>
where
    S: h3::quic::RecvStream,
{
    stream: h3::server::RequestStream<S, Bytes>,
    remaining: usize,
    telemetry: Http3ServerTelemetry,
    rejected: bool,
}

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

#[derive(Debug)]
enum H3RequestBodyError {
    LimitExceeded,
    Stream,
}

impl std::fmt::Display for H3RequestBodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LimitExceeded => f.write_str("HTTP/3 request body exceeded configured limit"),
            Self::Stream => f.write_str("HTTP/3 request body stream failed"),
        }
    }
}

impl std::error::Error for H3RequestBodyError {}

fn h3_request_to_axum(request: Request<()>, body: Body) -> Request<Body> {
    let (mut parts, ()) = request.into_parts();
    parts.version = http::Version::HTTP_3;
    Request::from_parts(parts, body)
}

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
