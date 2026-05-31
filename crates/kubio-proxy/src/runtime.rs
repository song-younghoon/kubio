use anyhow::Context;
use kubio_core::{unix_time_ms, EffectiveConfig};
use kubio_policy::PolicyEngine;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::route_hints::RouteHintLookup;

#[derive(Debug)]
pub struct ActiveRuntime {
    pub generation: u64,
    pub loaded_at_unix_ms: u64,
    pub config: Arc<EffectiveConfig>,
    pub policy: Arc<PolicyEngine>,
    pub(crate) route_hints: Arc<RouteHintLookup>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kubio_core::Mode;

    #[test]
    fn runtime_replace_increments_generation_and_preserves_old_snapshot() {
        let first_config = Arc::new(EffectiveConfig::default());
        let handle = RuntimeHandle::new(first_config).unwrap();
        let first = handle.load();
        let mut next_config = (*first.config).clone();
        next_config.mode = Mode::Auto;

        let second = handle.replace_config(Arc::new(next_config)).unwrap();

        assert_eq!(first.generation, 1);
        assert_eq!(first.config.mode, Mode::Watch);
        assert_eq!(second.generation, 2);
        assert_eq!(handle.load().config.mode, Mode::Auto);
    }
}

impl ActiveRuntime {
    fn build(generation: u64, config: Arc<EffectiveConfig>) -> anyhow::Result<Self> {
        let policy = Arc::new(PolicyEngine::new(&config));
        let route_hints = Arc::new(RouteHintLookup::new(&config.routes));
        Ok(Self {
            generation,
            loaded_at_unix_ms: unix_time_ms(),
            config,
            policy,
            route_hints,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeHandle {
    current: Arc<RwLock<Arc<ActiveRuntime>>>,
}

impl RuntimeHandle {
    pub fn new(config: Arc<EffectiveConfig>) -> anyhow::Result<Self> {
        Ok(Self {
            current: Arc::new(RwLock::new(Arc::new(ActiveRuntime::build(1, config)?))),
        })
    }

    pub fn load(&self) -> Arc<ActiveRuntime> {
        self.current.read().clone()
    }

    pub fn generation(&self) -> u64 {
        self.load().generation
    }

    pub fn active_config(&self) -> Arc<EffectiveConfig> {
        self.load().config.clone()
    }

    pub fn replace_config(
        &self,
        config: Arc<EffectiveConfig>,
    ) -> anyhow::Result<Arc<ActiveRuntime>> {
        let mut current = self.current.write();
        let next_generation = current.generation.saturating_add(1);
        let next = Arc::new(
            ActiveRuntime::build(next_generation, config)
                .with_context(|| format!("build runtime generation {next_generation}"))?,
        );
        *current = next.clone();
        Ok(next)
    }
}
