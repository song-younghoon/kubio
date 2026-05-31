use async_trait::async_trait;
use kubio_core::{
    ActiveConfigResponse, ConfigCheckRequest, ConfigReloadRequest, ConfigReloadResult,
    ConfigReloadSnapshot, EffectiveConfig,
};
use kubio_observe::Observer;
use kubio_store::CacheStore;
use std::sync::Arc;

#[async_trait]
pub trait ConfigReloadController: Send + Sync {
    fn active_config(&self) -> ActiveConfigResponse;
    fn reload_status(&self) -> ConfigReloadSnapshot;
    async fn reload_config(&self, request: ConfigReloadRequest) -> ConfigReloadResult;
    async fn check_config(&self, request: ConfigCheckRequest) -> ConfigReloadResult;
}

#[derive(Clone)]
pub struct DashboardState {
    pub config: Arc<EffectiveConfig>,
    pub observer: Arc<Observer>,
    pub store: Arc<dyn CacheStore>,
    pub reloader: Option<Arc<dyn ConfigReloadController>>,
}

impl DashboardState {
    pub fn active_config(&self) -> ActiveConfigResponse {
        self.reloader
            .as_ref()
            .map(|reloader| reloader.active_config())
            .unwrap_or_else(|| ActiveConfigResponse {
                generation: 1,
                loaded_at_unix_ms: 0,
                config: self.config.redacted(),
            })
    }

    pub fn reload_status(&self) -> ConfigReloadSnapshot {
        self.reloader
            .as_ref()
            .map(|reloader| reloader.reload_status())
            .unwrap_or_else(|| ConfigReloadSnapshot::startup(1, None))
    }
}
