use anyhow::Context;
use kubio_core::EffectiveConfig;
use kubio_observe::Observer;
use kubio_store::CacheStore;
#[cfg(feature = "experimental-http3")]
use kubio_transport::Http3OriginClient;
use kubio_transport::{origin_client_builder, origin_uses_http2_prior_knowledge};
use reqwest::Client;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::runtime::RuntimeHandle;

#[derive(Clone)]
pub struct ProxyState {
    pub config: Arc<EffectiveConfig>,
    pub runtime: RuntimeHandle,
    pub observer: Arc<Observer>,
    pub store: Arc<dyn CacheStore>,
    pub client: Client,
    pub fallback_client: Client,
    #[cfg(feature = "experimental-http3")]
    pub http3_origin_client: Option<Http3OriginClient>,
    pub(crate) in_flight: Arc<Semaphore>,
    pub(crate) panic_switch_was_active: Arc<AtomicBool>,
}

impl ProxyState {
    pub fn new(
        config: Arc<EffectiveConfig>,
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
        let runtime = RuntimeHandle::new(config.clone())?;
        observer.record_in_flight(0, max_in_flight_requests);
        Ok(Self {
            config,
            runtime,
            observer,
            store,
            client,
            fallback_client,
            #[cfg(feature = "experimental-http3")]
            http3_origin_client,
            in_flight: Arc::new(Semaphore::new(max_in_flight_requests)),
            panic_switch_was_active: Arc::new(AtomicBool::new(false)),
        })
    }
}
