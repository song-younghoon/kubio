use crate::config::{
    load_config_from_source, load_config_text_with_overrides, validate_config, StartupConfigSource,
    StartupOverrides,
};
use async_trait::async_trait;
use kubio_core::{
    diff_configs, unix_time_ms, ActiveConfigResponse, ConfigCheckRequest, ConfigDiff,
    ConfigReloadRequest, ConfigReloadResult, ConfigReloadSnapshot, EffectiveConfig, ReloadStatus,
    RouteId, RouteReloadAction,
};
use kubio_dashboard::ConfigReloadController;
use kubio_observe::{EventType, Observer};
use kubio_proxy::RuntimeHandle;
use kubio_store::{CacheStore, PurgeSelector};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;

pub(crate) struct ServeConfigReloader {
    source: Option<StartupConfigSource>,
    runtime: RuntimeHandle,
    observer: Arc<Observer>,
    store: Arc<dyn CacheStore>,
    attempts: AtomicU64,
    status: Mutex<ConfigReloadSnapshot>,
    reload_lock: AsyncMutex<()>,
}

impl ServeConfigReloader {
    pub(crate) fn new(
        source: Option<StartupConfigSource>,
        runtime: RuntimeHandle,
        observer: Arc<Observer>,
        store: Arc<dyn CacheStore>,
    ) -> Self {
        let source_label = source
            .as_ref()
            .map(|source| source.path.display().to_string());
        Self {
            source,
            runtime,
            observer,
            store,
            attempts: AtomicU64::new(0),
            status: Mutex::new(ConfigReloadSnapshot::startup(1, source_label)),
            reload_lock: AsyncMutex::new(()),
        }
    }

    pub(crate) async fn reload_from_source(&self) -> ConfigReloadResult {
        self.run_reload(false, None).await
    }

    async fn run_reload(
        &self,
        dry_run: bool,
        candidate_text: Option<String>,
    ) -> ConfigReloadResult {
        let _guard = self.reload_lock.lock().await;
        let attempt_id = self.attempts.fetch_add(1, Ordering::Relaxed) + 1;
        self.observer.record_config_reload_started(attempt_id);
        let attempted_at = unix_time_ms();
        let active = self.runtime.load();
        let active_generation = active.generation;

        let candidate = match self.load_candidate(candidate_text.as_deref()) {
            Ok(candidate) => Arc::new(candidate),
            Err(err) => {
                return self.finish(
                    ConfigReloadResult::simple(
                        if err.to_string().contains("no config source") {
                            ReloadStatus::NoConfigSource
                        } else {
                            ReloadStatus::ParseFailed
                        },
                        err.to_string(),
                        attempt_id,
                        active_generation,
                    ),
                    attempted_at,
                );
            }
        };
        if let Err(err) = validate_config(&candidate) {
            return self.finish(
                ConfigReloadResult::simple(
                    ReloadStatus::ValidationFailed,
                    err.to_string(),
                    attempt_id,
                    active_generation,
                ),
                attempted_at,
            );
        }

        let diff = diff_configs(&active.config, &candidate);
        if diff.has_restart_required() {
            return self.finish(
                result_from_diff(
                    ReloadStatus::RestartRequired,
                    "config contains restart-required changes",
                    attempt_id,
                    active_generation,
                    None,
                    &diff,
                    0,
                    0,
                ),
                attempted_at,
            );
        }

        if dry_run {
            return self.finish(
                result_from_diff(
                    ReloadStatus::DryRunOk,
                    "config can be reloaded",
                    attempt_id,
                    active_generation,
                    None,
                    &diff,
                    0,
                    0,
                ),
                attempted_at,
            );
        }

        let next_generation = active.generation.saturating_add(1);
        let (routes_demoted, cache_entries_purged) =
            match self.reconcile_state(&diff, next_generation).await {
                Ok(summary) => summary,
                Err(err) => {
                    return self.finish(
                        result_from_diff(
                            ReloadStatus::StateReconciliationFailed,
                            format!("state reconciliation failed: {err}"),
                            attempt_id,
                            active_generation,
                            None,
                            &diff,
                            0,
                            0,
                        ),
                        attempted_at,
                    );
                }
            };

        let next_runtime = match self.runtime.replace_config(candidate.clone()) {
            Ok(runtime) => runtime,
            Err(err) => {
                return self.finish(
                    result_from_diff(
                        ReloadStatus::InternalError,
                        err.to_string(),
                        attempt_id,
                        active_generation,
                        None,
                        &diff,
                        routes_demoted,
                        cache_entries_purged,
                    ),
                    attempted_at,
                );
            }
        };
        self.observer.apply_policy_config(
            candidate.policy.min_route_samples,
            candidate.policy.min_key_repeats,
            candidate.policy.min_shadow_validations,
            candidate.policy.adaptive_reuse.clone(),
            candidate.policy.response_header_equivalence.clone(),
        );
        self.observer.push_event(
            EventType::ConfigReloadStateReconciled,
            None,
            None,
            vec![kubio_core::DecisionReason::PolicyError],
            format!(
                "config reload state reconciled; demoted={routes_demoted} purged={cache_entries_purged}"
            ),
        );
        self.finish(
            result_from_diff(
                ReloadStatus::Applied,
                "config reload applied",
                attempt_id,
                next_runtime.generation,
                Some(active_generation),
                &diff,
                routes_demoted,
                cache_entries_purged,
            ),
            attempted_at,
        )
    }

    fn load_candidate(&self, text: Option<&str>) -> anyhow::Result<EffectiveConfig> {
        match (text, self.source.as_ref()) {
            (Some(text), Some(source)) => load_config_text_with_overrides(text, &source.overrides),
            (Some(text), None) => {
                load_config_text_with_overrides(text, &StartupOverrides::default())
            }
            (None, Some(source)) => load_config_from_source(source),
            (None, None) => anyhow::bail!("no config source is available for reload"),
        }
    }

    async fn reconcile_state(
        &self,
        diff: &ConfigDiff,
        next_generation: u64,
    ) -> anyhow::Result<(u64, u64)> {
        let mut purged_entries = 0;
        let routes_demoted = if diff.requires_global_cache_purge() {
            let purge = self.store.purge(PurgeSelector::All).await?;
            purged_entries += purge.purged_entries;
            if purge.purged_entries > 0 {
                self.observer.push_event(
                    EventType::ConfigReloadCachePurged,
                    None,
                    None,
                    vec![kubio_core::DecisionReason::StoreError],
                    format!(
                        "purged {} cache entries for config reload",
                        purge.purged_entries
                    ),
                );
            }
            self.observer.demote_all_routes_for_reload(
                next_generation,
                RouteReloadAction::Demoted,
                "reload changed global policy compatibility",
            )
        } else {
            let routes = diff.changed_or_removed_routes();
            let mut purged = 0;
            for route in &routes {
                let purge = self
                    .store
                    .purge(PurgeSelector::Route(route.clone()))
                    .await?;
                purged += purge.purged_entries;
            }
            purged_entries += purged;
            if purged > 0 {
                self.observer.push_event(
                    EventType::ConfigReloadCachePurged,
                    None,
                    None,
                    vec![kubio_core::DecisionReason::StoreError],
                    format!("purged {purged} route cache entries for config reload"),
                );
            }
            self.observer.demote_routes_for_reload(
                &routes,
                next_generation,
                RouteReloadAction::Demoted,
                "reload changed route hint compatibility",
            )
        };
        Ok((routes_demoted, purged_entries))
    }

    fn finish(&self, result: ConfigReloadResult, attempted_at: u64) -> ConfigReloadResult {
        {
            let mut status = self.status.lock().expect("reload status lock poisoned");
            status.active_generation = result.active_generation;
            status.apply_result(&result, attempted_at);
        }
        self.observer.record_config_reload_result(&result);
        result
    }
}

#[async_trait]
impl ConfigReloadController for ServeConfigReloader {
    fn active_config(&self) -> ActiveConfigResponse {
        let active = self.runtime.load();
        ActiveConfigResponse {
            generation: active.generation,
            loaded_at_unix_ms: active.loaded_at_unix_ms,
            config: active.config.redacted(),
        }
    }

    fn reload_status(&self) -> ConfigReloadSnapshot {
        self.status
            .lock()
            .expect("reload status lock poisoned")
            .clone()
    }

    async fn reload_config(&self, request: ConfigReloadRequest) -> ConfigReloadResult {
        self.run_reload(request.dry_run, None).await
    }

    async fn check_config(&self, request: ConfigCheckRequest) -> ConfigReloadResult {
        self.run_reload(true, request.config).await
    }
}

#[allow(clippy::too_many_arguments)]
fn result_from_diff(
    status: ReloadStatus,
    message: impl Into<String>,
    attempt_id: u64,
    active_generation: u64,
    previous_generation: Option<u64>,
    diff: &ConfigDiff,
    routes_demoted: u64,
    cache_entries_purged: u64,
) -> ConfigReloadResult {
    ConfigReloadResult {
        status,
        message: message.into(),
        attempt_id,
        active_generation,
        previous_generation,
        reloadable_changes: diff.reloadable_count(),
        restart_required: diff.restart_required_paths(),
        routes_added: diff.routes_added,
        routes_changed: diff.routes_changed,
        routes_removed: diff.routes_removed,
        routes_demoted,
        cache_entries_purged,
        diff: diff.entries.clone(),
    }
}

#[allow(dead_code)]
fn route_labels(routes: &[RouteId]) -> Vec<String> {
    routes.iter().map(RouteId::as_label).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config_from_source;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn valid_route_reload_applies_and_increments_generation() {
        let source = write_config(
            r#"
origin: "http://localhost:3000"
"#,
        );
        let initial = Arc::new(load_config_from_source(&source).unwrap());
        let reloader = test_reloader(Some(source.clone()), initial);
        std::fs::write(
            &source.path,
            r#"
origin: "http://localhost:3000"
routes:
  - match:
      method: GET
      path: "/api/products"
"#,
        )
        .unwrap();

        let result = reloader.reload_from_source().await;

        assert_eq!(result.status, ReloadStatus::Applied);
        assert_eq!(result.previous_generation, Some(1));
        assert_eq!(result.active_generation, 2);
        assert_eq!(result.routes_added, 1);
        let _ = std::fs::remove_file(source.path);
    }

    #[tokio::test]
    async fn restart_required_reload_keeps_active_generation() {
        let source = write_config(
            r#"
origin: "http://localhost:3000"
"#,
        );
        let initial = Arc::new(load_config_from_source(&source).unwrap());
        let reloader = test_reloader(Some(source.clone()), initial);
        std::fs::write(
            &source.path,
            r#"
origin: "http://localhost:3000"
server:
  listen: "127.0.0.1:8181"
"#,
        )
        .unwrap();

        let result = reloader.reload_from_source().await;

        assert_eq!(result.status, ReloadStatus::RestartRequired);
        assert_eq!(result.active_generation, 1);
        assert!(result
            .restart_required
            .contains(&"server.listen".to_string()));
        let _ = std::fs::remove_file(source.path);
    }

    #[tokio::test]
    async fn reload_without_source_reports_no_config_source() {
        let initial = Arc::new(EffectiveConfig::default());
        let reloader = test_reloader(None, initial);

        let result = reloader.reload_from_source().await;

        assert_eq!(result.status, ReloadStatus::NoConfigSource);
        assert_eq!(result.active_generation, 1);
    }

    fn test_reloader(
        source: Option<StartupConfigSource>,
        config: Arc<EffectiveConfig>,
    ) -> ServeConfigReloader {
        let runtime = RuntimeHandle::new(config.clone()).unwrap();
        let observer = Arc::new(Observer::with_policy_config(
            config.observability.max_routes,
            config.observability.max_keys,
            config.observability.max_events,
            config.policy.min_route_samples,
            config.policy.min_key_repeats,
            config.policy.min_shadow_validations,
            config.policy.adaptive_reuse.clone(),
            config.policy.response_header_equivalence.clone(),
        ));
        let store: Arc<dyn CacheStore> = Arc::new(kubio_store::MemoryStore::new(&config.storage));
        ServeConfigReloader::new(source, runtime, observer, store)
    }

    fn write_config(text: &str) -> StartupConfigSource {
        let path = temp_path();
        std::fs::write(&path, text).unwrap();
        StartupConfigSource {
            path,
            overrides: StartupOverrides::default(),
        }
    }

    fn temp_path() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "kubio-reload-test-{}-{suffix}.yml",
            std::process::id()
        ))
    }
}
