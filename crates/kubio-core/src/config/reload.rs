use super::EffectiveConfig;
use crate::{RouteHintConfig, RouteId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigChangeClass {
    Reloadable,
    RestartRequired,
}

impl ConfigChangeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reloadable => "reloadable",
            Self::RestartRequired => "restart_required",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDiffEntry {
    pub path: String,
    pub class: ConfigChangeClass,
    pub summary: String,
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDiffEntry {
    pub route_id: RouteId,
    pub action: RouteReloadAction,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDiff {
    pub entries: Vec<ConfigDiffEntry>,
    pub routes: Vec<RouteDiffEntry>,
    pub routes_added: u64,
    pub routes_changed: u64,
    pub routes_removed: u64,
}

impl ConfigDiff {
    pub fn reloadable_count(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| entry.class == ConfigChangeClass::Reloadable)
            .count() as u64
    }

    pub fn restart_required_paths(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.class == ConfigChangeClass::RestartRequired)
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub fn has_restart_required(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.class == ConfigChangeClass::RestartRequired)
    }

    pub fn changed_or_removed_routes(&self) -> Vec<RouteId> {
        self.routes
            .iter()
            .filter(|entry| {
                matches!(
                    entry.action,
                    RouteReloadAction::Changed | RouteReloadAction::Removed
                )
            })
            .map(|entry| entry.route_id.clone())
            .collect()
    }

    pub fn requires_global_cache_purge(&self) -> bool {
        self.entries.iter().any(|entry| {
            entry.class == ConfigChangeClass::Reloadable
                && matches!(
                    entry.path.as_str(),
                    "policy.respect_origin_headers"
                        | "policy.protect_authorization"
                        | "policy.protect_cookies"
                        | "policy.protect_set_cookie"
                        | "policy.max_object_size"
                        | "policy.max_fingerprint_body_size"
                        | "policy.revalidation"
                        | "policy.stale_if_error"
                        | "policy.query_intelligence"
                        | "policy.response_header_equivalence"
                        | "policy.adaptive_reuse"
                )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadStatus {
    Applied,
    DryRunOk,
    ParseFailed,
    ValidationFailed,
    RestartRequired,
    StateReconciliationFailed,
    NoConfigSource,
    Unauthorized,
    InternalError,
}

impl ReloadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::DryRunOk => "dry_run_ok",
            Self::ParseFailed => "parse_failed",
            Self::ValidationFailed => "validation_failed",
            Self::RestartRequired => "restart_required",
            Self::StateReconciliationFailed => "state_reconciliation_failed",
            Self::NoConfigSource => "no_config_source",
            Self::Unauthorized => "unauthorized",
            Self::InternalError => "internal_error",
        }
    }
}

impl std::fmt::Display for ReloadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteReloadAction {
    Unchanged,
    Added,
    Changed,
    Removed,
    Demoted,
    Purged,
    Retained,
    RequiresRevalidation,
}

impl std::fmt::Display for RouteReloadAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Unchanged => "unchanged",
            Self::Added => "added",
            Self::Changed => "changed",
            Self::Removed => "removed",
            Self::Demoted => "demoted",
            Self::Purged => "purged",
            Self::Retained => "retained",
            Self::RequiresRevalidation => "requires_revalidation",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveConfigResponse {
    pub generation: u64,
    pub loaded_at_unix_ms: u64,
    pub config: super::RedactedConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigReloadRequest {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigCheckRequest {
    #[serde(default)]
    pub config: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigReloadResult {
    pub status: ReloadStatus,
    pub message: String,
    pub attempt_id: u64,
    pub active_generation: u64,
    pub previous_generation: Option<u64>,
    pub reloadable_changes: u64,
    pub restart_required: Vec<String>,
    pub routes_added: u64,
    pub routes_changed: u64,
    pub routes_removed: u64,
    pub routes_demoted: u64,
    pub cache_entries_purged: u64,
    pub diff: Vec<ConfigDiffEntry>,
}

impl ConfigReloadResult {
    pub fn simple(
        status: ReloadStatus,
        message: impl Into<String>,
        attempt_id: u64,
        active_generation: u64,
    ) -> Self {
        Self {
            status,
            message: message.into(),
            attempt_id,
            active_generation,
            previous_generation: None,
            reloadable_changes: 0,
            restart_required: Vec::new(),
            routes_added: 0,
            routes_changed: 0,
            routes_removed: 0,
            routes_demoted: 0,
            cache_entries_purged: 0,
            diff: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigReloadSnapshot {
    pub active_generation: u64,
    pub startup_generation: u64,
    pub config_source: Option<String>,
    pub last_attempt_id: Option<u64>,
    pub last_attempt_at_unix_ms: Option<u64>,
    pub last_status: Option<ReloadStatus>,
    pub last_message: Option<String>,
    pub last_reloadable_change_count: u64,
    pub last_restart_required_count: u64,
    pub last_routes_added: u64,
    pub last_routes_changed: u64,
    pub last_routes_removed: u64,
    pub last_routes_demoted: u64,
    pub last_cache_entries_purged: u64,
}

impl ConfigReloadSnapshot {
    pub fn startup(active_generation: u64, config_source: Option<String>) -> Self {
        Self {
            active_generation,
            startup_generation: 1,
            config_source,
            last_attempt_id: None,
            last_attempt_at_unix_ms: None,
            last_status: None,
            last_message: None,
            last_reloadable_change_count: 0,
            last_restart_required_count: 0,
            last_routes_added: 0,
            last_routes_changed: 0,
            last_routes_removed: 0,
            last_routes_demoted: 0,
            last_cache_entries_purged: 0,
        }
    }

    pub fn apply_result(&mut self, result: &ConfigReloadResult, attempted_at_unix_ms: u64) {
        self.active_generation = result.active_generation;
        self.last_attempt_id = Some(result.attempt_id);
        self.last_attempt_at_unix_ms = Some(attempted_at_unix_ms);
        self.last_status = Some(result.status);
        self.last_message = Some(result.message.clone());
        self.last_reloadable_change_count = result.reloadable_changes;
        self.last_restart_required_count = result.restart_required.len() as u64;
        self.last_routes_added = result.routes_added;
        self.last_routes_changed = result.routes_changed;
        self.last_routes_removed = result.routes_removed;
        self.last_routes_demoted = result.routes_demoted;
        self.last_cache_entries_purged = result.cache_entries_purged;
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteReloadSnapshot {
    pub last_config_generation: u64,
    pub last_reload_action: Option<RouteReloadAction>,
    pub last_reload_reason: Option<String>,
}

pub fn diff_configs(previous: &EffectiveConfig, next: &EffectiveConfig) -> ConfigDiff {
    let mut diff = ConfigDiff::default();

    push_change(
        &mut diff,
        "server.listen",
        previous.server.listen,
        next.server.listen,
        ConfigChangeClass::RestartRequired,
        "listener address changed",
    );
    push_change(
        &mut diff,
        "server.origin_timeout",
        previous.server.origin_timeout,
        next.server.origin_timeout,
        ConfigChangeClass::RestartRequired,
        "origin timeout changed",
    );
    push_change(
        &mut diff,
        "server.tls",
        &previous.server.tls,
        &next.server.tls,
        ConfigChangeClass::RestartRequired,
        "TLS configuration changed",
    );
    push_change(
        &mut diff,
        "server.protocols",
        &previous.server.protocols,
        &next.server.protocols,
        ConfigChangeClass::RestartRequired,
        "server protocol topology changed",
    );
    push_change(
        &mut diff,
        "server.http2",
        &previous.server.http2,
        &next.server.http2,
        ConfigChangeClass::RestartRequired,
        "HTTP/2 runtime settings changed",
    );
    push_change(
        &mut diff,
        "server.http3",
        &previous.server.http3,
        &next.server.http3,
        ConfigChangeClass::RestartRequired,
        "HTTP/3 runtime settings changed",
    );
    push_change(
        &mut diff,
        "origin",
        previous.origin.as_str(),
        next.origin.as_str(),
        ConfigChangeClass::RestartRequired,
        "origin URL changed",
    );
    push_change(
        &mut diff,
        "origin_protocol",
        &previous.origin_protocol,
        &next.origin_protocol,
        ConfigChangeClass::RestartRequired,
        "origin protocol settings changed",
    );
    push_change(
        &mut diff,
        "mode",
        previous.mode,
        next.mode,
        ConfigChangeClass::Reloadable,
        "runtime mode changed",
    );
    push_change(
        &mut diff,
        "freshness",
        previous.freshness,
        next.freshness,
        ConfigChangeClass::Reloadable,
        "freshness profile changed",
    );
    push_change(
        &mut diff,
        "dashboard.enabled",
        previous.dashboard.enabled,
        next.dashboard.enabled,
        ConfigChangeClass::RestartRequired,
        "dashboard enabled state changed",
    );
    push_change(
        &mut diff,
        "dashboard.listen",
        previous.dashboard.listen,
        next.dashboard.listen,
        ConfigChangeClass::RestartRequired,
        "dashboard listener changed",
    );
    push_change(
        &mut diff,
        "dashboard.allow_public",
        previous.dashboard.allow_public,
        next.dashboard.allow_public,
        ConfigChangeClass::RestartRequired,
        "dashboard public binding policy changed",
    );
    push_change(
        &mut diff,
        "dashboard.admin_api",
        previous.dashboard.admin_api,
        next.dashboard.admin_api,
        ConfigChangeClass::RestartRequired,
        "dashboard admin API setting changed",
    );
    diff_policy(previous, next, &mut diff);
    diff_storage(previous, next, &mut diff);
    diff_performance(previous, next, &mut diff);
    diff_observability(previous, next, &mut diff);
    diff_routes(previous, next, &mut diff);
    push_change(
        &mut diff,
        "debug_headers",
        previous.debug_headers,
        next.debug_headers,
        ConfigChangeClass::Reloadable,
        "debug header setting changed",
    );
    push_change(
        &mut diff,
        "panic_file",
        &previous.panic_file,
        &next.panic_file,
        ConfigChangeClass::Reloadable,
        "panic switch file changed",
    );
    if previous.admin_token != next.admin_token {
        diff.entries.push(ConfigDiffEntry {
            path: "admin_token".to_string(),
            class: ConfigChangeClass::RestartRequired,
            summary: "admin token changed; value redacted".to_string(),
            secret: true,
        });
    }

    diff
}

pub fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn diff_policy(previous: &EffectiveConfig, next: &EffectiveConfig, diff: &mut ConfigDiff) {
    push_change(
        diff,
        "policy.respect_origin_headers",
        previous.policy.respect_origin_headers,
        next.policy.respect_origin_headers,
        ConfigChangeClass::Reloadable,
        "origin cache-header policy changed",
    );
    push_change(
        diff,
        "policy.protect_authorization",
        previous.policy.protect_authorization,
        next.policy.protect_authorization,
        ConfigChangeClass::Reloadable,
        "authorization protection policy changed",
    );
    push_change(
        diff,
        "policy.protect_cookies",
        previous.policy.protect_cookies,
        next.policy.protect_cookies,
        ConfigChangeClass::Reloadable,
        "cookie protection policy changed",
    );
    push_change(
        diff,
        "policy.protect_set_cookie",
        previous.policy.protect_set_cookie,
        next.policy.protect_set_cookie,
        ConfigChangeClass::Reloadable,
        "Set-Cookie protection policy changed",
    );
    push_change(
        diff,
        "policy.max_object_size",
        previous.policy.max_object_size,
        next.policy.max_object_size,
        ConfigChangeClass::Reloadable,
        "policy object size limit changed",
    );
    push_change(
        diff,
        "policy.max_fingerprint_body_size",
        previous.policy.max_fingerprint_body_size,
        next.policy.max_fingerprint_body_size,
        ConfigChangeClass::Reloadable,
        "fingerprint body limit changed",
    );
    push_change(
        diff,
        "policy.max_request_body_size",
        previous.policy.max_request_body_size,
        next.policy.max_request_body_size,
        ConfigChangeClass::Reloadable,
        "request body policy limit changed",
    );
    push_change(
        diff,
        "policy.min_route_samples",
        previous.policy.min_route_samples,
        next.policy.min_route_samples,
        ConfigChangeClass::Reloadable,
        "route sample threshold changed",
    );
    push_change(
        diff,
        "policy.min_key_repeats",
        previous.policy.min_key_repeats,
        next.policy.min_key_repeats,
        ConfigChangeClass::Reloadable,
        "key repeat threshold changed",
    );
    push_change(
        diff,
        "policy.min_shadow_validations",
        previous.policy.min_shadow_validations,
        next.policy.min_shadow_validations,
        ConfigChangeClass::Reloadable,
        "shadow validation threshold changed",
    );
    push_change(
        diff,
        "policy.max_shadow_mismatch_rate",
        previous.policy.max_shadow_mismatch_rate,
        next.policy.max_shadow_mismatch_rate,
        ConfigChangeClass::Reloadable,
        "shadow mismatch threshold changed",
    );
    push_change(
        diff,
        "policy.revalidation",
        &previous.policy.revalidation,
        &next.policy.revalidation,
        ConfigChangeClass::Reloadable,
        "revalidation policy changed",
    );
    push_change(
        diff,
        "policy.stale_if_error",
        &previous.policy.stale_if_error,
        &next.policy.stale_if_error,
        ConfigChangeClass::Reloadable,
        "stale-if-error policy changed",
    );
    push_change(
        diff,
        "policy.query_intelligence",
        &previous.policy.query_intelligence,
        &next.policy.query_intelligence,
        ConfigChangeClass::Reloadable,
        "query intelligence policy changed",
    );
    push_change(
        diff,
        "policy.response_header_equivalence",
        &previous.policy.response_header_equivalence,
        &next.policy.response_header_equivalence,
        ConfigChangeClass::Reloadable,
        "response-header equivalence policy changed",
    );
    push_change(
        diff,
        "policy.adaptive_reuse",
        &previous.policy.adaptive_reuse,
        &next.policy.adaptive_reuse,
        ConfigChangeClass::Reloadable,
        "adaptive reuse policy changed",
    );
}

fn diff_storage(previous: &EffectiveConfig, next: &EffectiveConfig, diff: &mut ConfigDiff) {
    push_change(
        diff,
        "storage.kind",
        previous.storage.kind.as_str(),
        next.storage.kind.as_str(),
        ConfigChangeClass::RestartRequired,
        "storage backend changed",
    );
    push_change(
        diff,
        "storage.max_size",
        previous.storage.max_size,
        next.storage.max_size,
        ConfigChangeClass::RestartRequired,
        "storage capacity changed",
    );
    push_change(
        diff,
        "storage.max_object_size",
        previous.storage.max_object_size,
        next.storage.max_object_size,
        ConfigChangeClass::RestartRequired,
        "store object limit changed",
    );
    push_change(
        diff,
        "storage.path",
        &previous.storage.path,
        &next.storage.path,
        ConfigChangeClass::RestartRequired,
        "storage path changed",
    );
    push_change(
        diff,
        "storage.sync",
        previous.storage.sync,
        next.storage.sync,
        ConfigChangeClass::RestartRequired,
        "storage sync policy changed",
    );
}

fn diff_performance(previous: &EffectiveConfig, next: &EffectiveConfig, diff: &mut ConfigDiff) {
    push_change(
        diff,
        "performance",
        &previous.performance,
        &next.performance,
        ConfigChangeClass::RestartRequired,
        "runtime performance settings changed",
    );
}

fn diff_observability(previous: &EffectiveConfig, next: &EffectiveConfig, diff: &mut ConfigDiff) {
    push_change(
        diff,
        "observability.metrics",
        previous.observability.metrics,
        next.observability.metrics,
        ConfigChangeClass::RestartRequired,
        "metrics endpoint setting changed",
    );
    push_change(
        diff,
        "observability.metrics_path",
        previous.observability.metrics_path.as_str(),
        next.observability.metrics_path.as_str(),
        ConfigChangeClass::RestartRequired,
        "metrics endpoint path changed",
    );
    push_change(
        diff,
        "observability.tracing",
        previous.observability.tracing,
        next.observability.tracing,
        ConfigChangeClass::Reloadable,
        "tracing setting changed",
    );
    push_change(
        diff,
        "observability.max_routes",
        previous.observability.max_routes,
        next.observability.max_routes,
        ConfigChangeClass::RestartRequired,
        "observer route limit changed",
    );
    push_change(
        diff,
        "observability.max_keys",
        previous.observability.max_keys,
        next.observability.max_keys,
        ConfigChangeClass::RestartRequired,
        "observer key limit changed",
    );
    push_change(
        diff,
        "observability.max_events",
        previous.observability.max_events,
        next.observability.max_events,
        ConfigChangeClass::RestartRequired,
        "observer event limit changed",
    );
}

fn diff_routes(previous: &EffectiveConfig, next: &EffectiveConfig, diff: &mut ConfigDiff) {
    let previous_routes = route_map(&previous.routes);
    let next_routes = route_map(&next.routes);
    let mut all = BTreeSet::new();
    all.extend(previous_routes.keys().cloned());
    all.extend(next_routes.keys().cloned());

    for route_id in all {
        match (previous_routes.get(&route_id), next_routes.get(&route_id)) {
            (None, Some(_)) => {
                diff.routes_added += 1;
                diff.routes.push(RouteDiffEntry {
                    route_id,
                    action: RouteReloadAction::Added,
                });
            }
            (Some(_), None) => {
                diff.routes_removed += 1;
                diff.routes.push(RouteDiffEntry {
                    route_id,
                    action: RouteReloadAction::Removed,
                });
            }
            (Some(previous), Some(next)) if previous != next => {
                diff.routes_changed += 1;
                diff.routes.push(RouteDiffEntry {
                    route_id,
                    action: RouteReloadAction::Changed,
                });
            }
            _ => {}
        }
    }

    if diff.routes_added > 0 || diff.routes_changed > 0 || diff.routes_removed > 0 {
        let summary = format!(
            "{} added, {} changed, {} removed",
            diff.routes_added, diff.routes_changed, diff.routes_removed
        );
        diff.entries.push(ConfigDiffEntry {
            path: "routes".to_string(),
            class: ConfigChangeClass::Reloadable,
            summary,
            secret: false,
        });
    }
}

fn route_map(routes: &[RouteHintConfig]) -> BTreeMap<RouteId, &RouteHintConfig> {
    routes
        .iter()
        .map(|route| {
            (
                RouteId::new(
                    route.route_match.method.to_ascii_uppercase(),
                    route.route_match.path.clone(),
                ),
                route,
            )
        })
        .collect()
}

fn push_change<T: PartialEq>(
    diff: &mut ConfigDiff,
    path: &str,
    previous: T,
    next: T,
    class: ConfigChangeClass,
    summary: &str,
) {
    if previous != next {
        diff.entries.push(ConfigDiffEntry {
            path: path.to_string(),
            class,
            summary: summary.to_string(),
            secret: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RouteFreshnessConfig, RouteMatchConfig, RouteQueryConfig, RouteResponseHeadersConfig,
        RouteSafetyConfig, RouteStaleIfErrorConfig, RouteVaryConfig,
    };

    #[test]
    fn diff_classifies_route_changes_as_reloadable() {
        let previous = EffectiveConfig::default();
        let mut next = previous.clone();
        next.routes.push(RouteHintConfig {
            name: Some("products".to_string()),
            route_match: RouteMatchConfig {
                method: "GET".to_string(),
                path: "/api/products".to_string(),
            },
            query: RouteQueryConfig {
                ignore: vec!["utm_*".to_string()],
                ..Default::default()
            },
            freshness: RouteFreshnessConfig::default(),
            vary: RouteVaryConfig::default(),
            stale_if_error: RouteStaleIfErrorConfig::default(),
            safety: RouteSafetyConfig::default(),
            response_headers: RouteResponseHeadersConfig::default(),
        });

        let diff = diff_configs(&previous, &next);

        assert_eq!(diff.reloadable_count(), 1);
        assert!(!diff.has_restart_required());
        assert_eq!(diff.routes_added, 1);
    }

    #[test]
    fn diff_classifies_listener_origin_storage_and_admin_token_as_restart_required() {
        let previous = EffectiveConfig::default();
        let mut next = previous.clone();
        next.server.listen = "127.0.0.1:7777".parse().unwrap();
        next.origin = "http://example.com".parse().unwrap();
        next.storage.kind = "disk".to_string();
        next.admin_token = Some("raw-secret".to_string());

        let diff = diff_configs(&previous, &next);
        let paths = diff.restart_required_paths();

        assert!(paths.contains(&"server.listen".to_string()));
        assert!(paths.contains(&"origin".to_string()));
        assert!(paths.contains(&"storage.kind".to_string()));
        assert!(paths.contains(&"admin_token".to_string()));
        assert!(
            diff.entries
                .iter()
                .find(|entry| entry.path == "admin_token")
                .unwrap()
                .secret
        );
        assert!(!format!("{diff:?}").contains("raw-secret"));
    }

    #[test]
    fn diff_rejects_mixed_reloadable_and_restart_required_changes() {
        let previous = EffectiveConfig::default();
        let mut next = previous.clone();
        next.mode = crate::Mode::Auto;
        next.server.listen = "127.0.0.1:7777".parse().unwrap();

        let diff = diff_configs(&previous, &next);

        assert_eq!(diff.reloadable_count(), 1);
        assert!(diff.has_restart_required());
    }
}
