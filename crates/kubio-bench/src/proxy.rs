use crate::args::BenchProtocol;
#[cfg(feature = "experimental-http3")]
use crate::h3::{tls_cert_path, tls_key_path, unused_udp_addr};
use anyhow::{bail, Result};
#[cfg(feature = "experimental-http3")]
use kubio_core::TlsConfig;
use kubio_core::{EffectiveConfig, Mode};
use kubio_observe::Observer;
use kubio_policy::PolicyEngine;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::MemoryStore;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use url::Url;

pub(crate) struct ManagedProxy {
    addr: SocketAddr,
    #[cfg(feature = "experimental-http3")]
    http3_addr: Option<SocketAddr>,
    pub(crate) observer: Arc<Observer>,
    pub(crate) store: Arc<MemoryStore>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ManagedProxy {
    pub(crate) async fn start(origin: Url, protocol: BenchProtocol) -> Result<Self> {
        let addr = unused_addr().await?;
        let defaults = EffectiveConfig::default();
        let mut server = defaults.server.clone();
        server.listen = addr;
        match protocol {
            BenchProtocol::H1 => {}
            BenchProtocol::H2 => {
                server.protocols.http2 = true;
                server.protocols.h2c = true;
            }
            BenchProtocol::H3 => {
                #[cfg(feature = "experimental-http3")]
                {
                    server.tls = Some(TlsConfig {
                        cert: tls_cert_path(),
                        key: tls_key_path(),
                    });
                    server.http3.enabled = true;
                    server.http3.listen = Some(unused_udp_addr()?);
                }
            }
        }
        let mut policy = defaults.policy.clone();
        policy.min_route_samples = 2;
        policy.min_key_repeats = 2;
        policy.min_shadow_validations = 1;
        let config = Arc::new(EffectiveConfig {
            origin,
            mode: Mode::Auto,
            server,
            policy,
            ..defaults
        });
        let observer = Arc::new(Observer::new(100, 100, 100, 2, 2, 1));
        let store = Arc::new(MemoryStore::new(&config.storage));
        let policy = Arc::new(PolicyEngine::new(&config));
        let state = ProxyState::new(config.clone(), policy, observer.clone(), store.clone())?;
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = run_proxy(state, async {
                let _ = rx.await;
            })
            .await;
        });
        wait_tcp_ready(addr).await?;
        Ok(Self {
            addr,
            #[cfg(feature = "experimental-http3")]
            http3_addr: config.server.http3.listen,
            observer,
            store,
            shutdown: Some(tx),
        })
    }

    pub(crate) fn http_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    #[cfg(feature = "experimental-http3")]
    pub(crate) fn http3_addr(&self) -> Option<SocketAddr> {
        self.http3_addr
    }
}

impl Drop for ManagedProxy {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn wait_tcp_ready(addr: SocketAddr) -> Result<()> {
    for _ in 0..50 {
        if TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    bail!("proxy did not become ready at {addr}")
}

async fn unused_addr() -> Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr)
}
