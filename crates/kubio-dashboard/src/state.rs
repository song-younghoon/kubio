use kubio_core::EffectiveConfig;
use kubio_observe::Observer;
use kubio_store::CacheStore;
use std::sync::Arc;

#[derive(Clone)]
pub struct DashboardState {
    pub config: Arc<EffectiveConfig>,
    pub observer: Arc<Observer>,
    pub store: Arc<dyn CacheStore>,
}
