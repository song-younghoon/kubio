use axum::body::Body;
use axum::Router;
use http::Request;
use hyper::body::Incoming as HyperIncoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder as HyperServerBuilder;
use hyper_util::service::TowerToHyperService;
use kubio_core::EffectiveConfig;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::util::ServiceExt;
use tracing::{debug, warn};

use crate::tls::tls_acceptor;

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

fn is_connection_accept_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
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
}
