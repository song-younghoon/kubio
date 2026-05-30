#[cfg(feature = "experimental-http3")]
use anyhow::Context;
use axum::routing::any;
use axum::Router;
#[cfg(feature = "experimental-http3")]
use kubio_observe::Http3ServerEvent as ObservedHttp3ServerEvent;
#[cfg(feature = "experimental-http3")]
use kubio_observe::Observer;
use kubio_transport::serve_http12_router;
#[cfg(feature = "experimental-http3")]
use kubio_transport::{
    serve_http3_router, Http3ServerEvent as TransportHttp3ServerEvent, Http3ServerTelemetry,
};
use std::future::Future;
#[cfg(feature = "experimental-http3")]
use std::sync::Arc;
#[cfg(feature = "experimental-http3")]
use tokio::sync::broadcast;

use crate::handler::proxy_handler;
use crate::state::ProxyState;

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
    config: Arc<kubio_core::EffectiveConfig>,
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
