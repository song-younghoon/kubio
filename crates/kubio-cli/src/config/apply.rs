use super::file::{
    FileConfig, FileHttp2Config, FileHttp3Config, FilePerformanceConfig, FilePolicyConfig,
};
use crate::args::ServeArgs;
use anyhow::{bail, Context, Result};
use kubio_core::{
    parse_duration, parse_size, EffectiveConfig, Http2Config, Http3ServerConfig, PerformanceConfig,
    PolicyConfig, RouteHintConfig, TlsConfig,
};
use std::path::PathBuf;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone)]
pub(crate) struct StartupConfigSource {
    pub(crate) path: PathBuf,
    pub(crate) overrides: StartupOverrides,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StartupOverrides {
    pub(crate) origin: Option<String>,
    pub(crate) listen: Option<std::net::SocketAddr>,
    pub(crate) dashboard: Option<std::net::SocketAddr>,
    pub(crate) mode: Option<String>,
    pub(crate) freshness: Option<String>,
    pub(crate) debug_headers: bool,
    pub(crate) panic_file: Option<PathBuf>,
}

impl StartupOverrides {
    pub(crate) fn from_serve_args(args: &ServeArgs) -> Self {
        Self {
            origin: args.origin.clone(),
            listen: args.listen,
            dashboard: args.dashboard,
            mode: args.mode.clone(),
            freshness: args.freshness.clone(),
            debug_headers: args.debug_headers,
            panic_file: args.panic_file.clone(),
        }
    }

    pub(crate) fn apply(&self, config: &mut EffectiveConfig) -> Result<()> {
        if let Some(origin) = self.origin.as_ref() {
            config.origin = Url::parse(origin).context("parse --to origin URL")?;
        }
        if let Some(listen) = self.listen {
            config.server.listen = listen;
        }
        if let Some(dashboard) = self.dashboard {
            config.dashboard.listen = dashboard;
        }
        if let Some(mode) = self.mode.as_ref() {
            config.mode = mode.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(freshness) = self.freshness.as_ref() {
            config.freshness = freshness.parse().map_err(anyhow::Error::msg)?;
        }
        if self.debug_headers {
            config.debug_headers = true;
        }
        if self.panic_file.is_some() {
            config.panic_file = self.panic_file.clone();
        }
        Ok(())
    }
}

pub(crate) fn load_config_for_serve(args: &ServeArgs) -> Result<EffectiveConfig> {
    let source = args.config.as_ref().map(|path| StartupConfigSource {
        path: path.clone(),
        overrides: StartupOverrides::from_serve_args(args),
    });
    let file = source
        .as_ref()
        .map(|source| load_config_file(&source.path))
        .transpose()?;

    let origin_set = args.origin.is_some()
        || file
            .as_ref()
            .and_then(|config| config.origin.as_ref())
            .is_some();
    if !origin_set {
        bail!("origin URL is required; pass --to <URL> or set origin in config");
    }

    let mut config = EffectiveConfig::default();
    if let Some(file) = file {
        apply_file_config(&mut config, file)?;
    }
    StartupOverrides::from_serve_args(args).apply(&mut config)?;

    Ok(config)
}

pub(crate) fn config_source_for_serve(args: &ServeArgs) -> Option<StartupConfigSource> {
    args.config.as_ref().map(|path| StartupConfigSource {
        path: path.clone(),
        overrides: StartupOverrides::from_serve_args(args),
    })
}

pub(crate) fn load_config_from_source(source: &StartupConfigSource) -> Result<EffectiveConfig> {
    let mut config = EffectiveConfig::default();
    apply_file_config(&mut config, load_config_file(&source.path)?)?;
    source.overrides.apply(&mut config)?;
    Ok(config)
}

pub(crate) fn load_config_text_with_overrides(
    text: &str,
    overrides: &StartupOverrides,
) -> Result<EffectiveConfig> {
    let mut config = EffectiveConfig::default();
    apply_file_config(&mut config, parse_config_text(text)?)?;
    overrides.apply(&mut config)?;
    Ok(config)
}

pub(crate) fn load_config_file(path: &PathBuf) -> Result<FileConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read config file {}", path.display()))?;
    parse_config_text(&text).with_context(|| format!("parse config file {}", path.display()))
}

pub(crate) fn parse_config_text(text: &str) -> Result<FileConfig> {
    serde_yaml::from_str(text).context("parse config")
}

pub(crate) fn apply_file_config(config: &mut EffectiveConfig, file: FileConfig) -> Result<()> {
    if let Some(origin) = file.origin {
        config.origin = Url::parse(&origin).context("parse config origin")?;
    }
    if let Some(mode) = file.mode {
        config.mode = mode.parse().map_err(anyhow::Error::msg)?;
    }
    if let Some(freshness) = file.freshness {
        config.freshness = freshness.parse().map_err(anyhow::Error::msg)?;
    }
    if let Some(server) = file.server {
        if let Some(listen) = server.listen {
            config.server.listen = listen.parse().context("parse server.listen")?;
        }
        if let Some(origin_timeout_ms) = server.origin_timeout_ms {
            if origin_timeout_ms == 0 {
                bail!("server.origin_timeout_ms must be greater than zero");
            }
            config.server.origin_timeout = Duration::from_millis(origin_timeout_ms);
        }
        if let Some(tls) = server.tls {
            config.server.tls = Some(TlsConfig {
                cert: PathBuf::from(tls.cert),
                key: PathBuf::from(tls.key),
            });
        }
        if let Some(protocols) = server.protocols {
            if let Some(http1) = protocols.http1 {
                config.server.protocols.http1 = http1;
            }
            if let Some(http2) = protocols.http2 {
                config.server.protocols.http2 = http2;
            }
            if let Some(h2c) = protocols.h2c {
                config.server.protocols.h2c = h2c;
            }
        }
        if let Some(http2) = server.http2 {
            apply_http2_config(&mut config.server.http2, http2)?;
        }
        if let Some(http3) = server.http3 {
            apply_http3_config(&mut config.server.http3, http3)?;
        }
    }
    if let Some(origin_protocol) = file.origin_protocol {
        if let Some(preferred) = origin_protocol.preferred {
            config.origin_protocol.preferred = preferred.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(fallback) = origin_protocol.fallback {
            config.origin_protocol.fallback = fallback;
        }
        if let Some(http2_prior_knowledge) = origin_protocol.http2_prior_knowledge {
            config.origin_protocol.http2_prior_knowledge = http2_prior_knowledge;
        }
        if let Some(http3_experimental) = origin_protocol.http3_experimental {
            config.origin_protocol.http3_experimental = http3_experimental;
        }
        if let Some(http3_max_idle_connections) = origin_protocol.http3_max_idle_connections {
            config.origin_protocol.http3_max_idle_connections = http3_max_idle_connections;
        }
        if let Some(http3_idle_timeout) = origin_protocol.http3_idle_timeout {
            config.origin_protocol.http3_idle_timeout =
                parse_duration(&http3_idle_timeout).map_err(anyhow::Error::msg)?;
        }
        if let Some(http3_ca_certs) = origin_protocol.http3_ca_certs {
            config.origin_protocol.http3_ca_certs =
                http3_ca_certs.into_iter().map(PathBuf::from).collect();
        }
    }
    if let Some(dashboard) = file.dashboard {
        if let Some(enabled) = dashboard.enabled {
            config.dashboard.enabled = enabled;
        }
        if let Some(listen) = dashboard.listen {
            config.dashboard.listen = listen.parse().context("parse dashboard.listen")?;
        }
        if let Some(allow_public) = dashboard.allow_public {
            config.dashboard.allow_public = allow_public;
        }
        if let Some(admin_api) = dashboard.admin_api {
            config.dashboard.admin_api = admin_api;
        }
    }
    if let Some(policy) = file.policy {
        apply_policy_config(&mut config.policy, policy)?;
    }
    if let Some(storage) = file.storage {
        if let Some(kind) = storage.kind {
            config.storage.kind = kind;
        }
        if let Some(max_size) = storage.max_size {
            config.storage.max_size = parse_size(&max_size).map_err(anyhow::Error::msg)?;
        }
        if let Some(max_object_size) = storage.max_object_size {
            config.storage.max_object_size =
                parse_size(&max_object_size).map_err(anyhow::Error::msg)?;
            config.policy.max_object_size = config.storage.max_object_size;
        }
        if let Some(path) = storage.path {
            config.storage.path = Some(PathBuf::from(path));
        }
        if let Some(sync) = storage.sync {
            config.storage.sync = sync;
        }
    }
    if let Some(performance) = file.performance {
        apply_performance_config(&mut config.performance, performance)?;
    }
    if let Some(observability) = file.observability {
        if let Some(metrics) = observability.metrics {
            config.observability.metrics = metrics;
        }
        if let Some(metrics_path) = observability.metrics_path {
            config.observability.metrics_path = metrics_path;
        }
        if let Some(tracing) = observability.tracing {
            config.observability.tracing = tracing;
        }
    }
    if let Some(debug_headers) = file.debug_headers {
        config.debug_headers = debug_headers;
    }
    if let Some(panic_file) = file.panic_file {
        config.panic_file = Some(PathBuf::from(panic_file));
    }
    if let Some(admin_token) = file.admin_token {
        config.admin_token = Some(admin_token);
    }
    if let Some(routes) = file.routes {
        config.routes = routes
            .into_iter()
            .map(RouteHintConfig::try_from)
            .collect::<Result<Vec<_>>>()?;
    }
    Ok(())
}

fn apply_http2_config(config: &mut Http2Config, http2: FileHttp2Config) -> Result<()> {
    if let Some(max_concurrent_streams) = http2.max_concurrent_streams {
        config.max_concurrent_streams = max_concurrent_streams;
    }
    if let Some(initial_stream_window_size) = http2.initial_stream_window_size {
        config.initial_stream_window_size = parse_size_u32(&initial_stream_window_size)?;
    }
    if let Some(initial_connection_window_size) = http2.initial_connection_window_size {
        config.initial_connection_window_size = parse_size_u32(&initial_connection_window_size)?;
    }
    if let Some(keepalive_interval) = http2.keepalive_interval {
        config.keepalive_interval =
            Some(parse_duration(&keepalive_interval).map_err(anyhow::Error::msg)?);
    }
    if let Some(keepalive_timeout) = http2.keepalive_timeout {
        config.keepalive_timeout =
            parse_duration(&keepalive_timeout).map_err(anyhow::Error::msg)?;
    }
    if let Some(max_header_list_size) = http2.max_header_list_size {
        config.max_header_list_size =
            parse_size(&max_header_list_size).map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

fn parse_size_u32(value: &str) -> Result<u32> {
    let parsed = parse_size(value).map_err(anyhow::Error::msg)?;
    u32::try_from(parsed).context("size exceeds u32 limit")
}

fn parse_size_u16(value: &str) -> Result<u16> {
    let parsed = parse_size(value).map_err(anyhow::Error::msg)?;
    u16::try_from(parsed).context("size exceeds u16 limit")
}

fn apply_http3_config(config: &mut Http3ServerConfig, http3: FileHttp3Config) -> Result<()> {
    if let Some(enabled) = http3.enabled {
        config.enabled = enabled;
    }
    if let Some(listen) = http3.listen {
        config.listen = Some(listen.parse().context("parse server.http3.listen")?);
    }
    if let Some(advertise) = http3.advertise {
        config.advertise = advertise;
    }
    if let Some(authorities) = http3.authorities {
        config.authorities = authorities
            .into_iter()
            .map(|authority| authority.trim().to_ascii_lowercase())
            .collect();
    }
    if let Some(alt_svc_ma) = http3.alt_svc_ma {
        config.alt_svc_ma = parse_duration(&alt_svc_ma).map_err(anyhow::Error::msg)?;
    }
    if let Some(max_concurrent_streams) = http3.max_concurrent_streams {
        config.max_concurrent_streams = max_concurrent_streams;
    }
    if let Some(max_field_section_size) = http3.max_field_section_size {
        config.max_field_section_size =
            parse_size(&max_field_section_size).map_err(anyhow::Error::msg)?;
    }
    if let Some(qpack_max_table_capacity) = http3.qpack_max_table_capacity {
        config.qpack_max_table_capacity =
            parse_size(&qpack_max_table_capacity).map_err(anyhow::Error::msg)?;
    }
    if let Some(max_udp_payload_size) = http3.max_udp_payload_size {
        config.max_udp_payload_size = parse_size_u16(&max_udp_payload_size)?;
    }
    if let Some(idle_timeout) = http3.idle_timeout {
        config.idle_timeout = parse_duration(&idle_timeout).map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

fn apply_performance_config(
    config: &mut PerformanceConfig,
    performance: FilePerformanceConfig,
) -> Result<()> {
    if let Some(max_in_flight_requests) = performance.max_in_flight_requests {
        config.max_in_flight_requests = max_in_flight_requests;
    }
    if let Some(max_buffered_response_size) = performance.max_buffered_response_size {
        config.max_buffered_response_size =
            parse_size(&max_buffered_response_size).map_err(anyhow::Error::msg)?;
    }
    if let Some(stream_unstoreable_bodies) = performance.stream_unstoreable_bodies {
        config.stream_unstoreable_bodies = stream_unstoreable_bodies;
    }
    if let Some(observer_shards) = performance.observer_shards {
        config.observer_shards = observer_shards;
    }
    if let Some(async_disk_writes) = performance.async_disk_writes {
        config.async_disk_writes = async_disk_writes;
    }
    if let Some(origin_pool_max_idle_per_host) = performance.origin_pool_max_idle_per_host {
        config.origin_pool_max_idle_per_host = origin_pool_max_idle_per_host;
    }
    if let Some(origin_pool_idle_timeout) = performance.origin_pool_idle_timeout {
        config.origin_pool_idle_timeout =
            parse_duration(&origin_pool_idle_timeout).map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

fn apply_policy_config(config: &mut PolicyConfig, policy: FilePolicyConfig) -> Result<()> {
    if let Some(value) = policy.respect_origin_headers {
        config.respect_origin_headers = value;
    }
    if let Some(value) = policy.protect_authorization {
        config.protect_authorization = value;
    }
    if let Some(value) = policy.protect_cookies {
        config.protect_cookies = value;
    }
    if let Some(value) = policy.protect_set_cookie {
        config.protect_set_cookie = value;
    }
    if let Some(value) = policy.max_object_size {
        config.max_object_size = parse_size(&value).map_err(anyhow::Error::msg)?;
    }
    if let Some(value) = policy.max_fingerprint_body_size {
        config.max_fingerprint_body_size = parse_size(&value).map_err(anyhow::Error::msg)?;
    }
    if let Some(value) = policy.min_route_samples {
        config.min_route_samples = value;
    }
    if let Some(value) = policy.min_key_repeats {
        config.min_key_repeats = value;
    }
    if let Some(value) = policy.min_shadow_validations {
        config.min_shadow_validations = value;
    }
    if let Some(value) = policy.max_shadow_mismatch_rate {
        config.max_shadow_mismatch_rate = value;
    }
    if let Some(revalidation) = policy.revalidation {
        if let Some(enabled) = revalidation.enabled {
            config.revalidation.enabled = enabled;
        }
        if let Some(prefer_etag) = revalidation.prefer_etag {
            config.revalidation.prefer_etag = prefer_etag;
        }
        if let Some(max_validator_length) = revalidation.max_validator_length {
            config.revalidation.max_validator_length = max_validator_length;
        }
    }
    if let Some(stale_if_error) = policy.stale_if_error {
        if let Some(mode) = stale_if_error.mode {
            config.stale_if_error.mode = mode.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(max_stale) = stale_if_error.max_stale {
            config.stale_if_error.max_stale =
                parse_duration(&max_stale).map_err(anyhow::Error::msg)?;
        }
    }
    if let Some(query_intelligence) = policy.query_intelligence {
        if let Some(enabled) = query_intelligence.enabled {
            config.query_intelligence.enabled = enabled;
        }
        if let Some(auto_ignore) = query_intelligence.auto_ignore {
            config.query_intelligence.auto_ignore = auto_ignore;
        }
    }
    if let Some(response_header_equivalence) = policy.response_header_equivalence {
        config.response_header_equivalence = response_header_equivalence;
    }
    if let Some(adaptive_reuse) = policy.adaptive_reuse {
        if let Some(enabled) = adaptive_reuse.enabled {
            config.adaptive_reuse.enabled = enabled;
        }
        if let Some(key_validation) = adaptive_reuse.key_validation {
            config.adaptive_reuse.key_validation = key_validation;
        }
        if let Some(public_object) = adaptive_reuse.public_object {
            config.adaptive_reuse.public_object = public_object;
        }
        if let Some(origin_public_fast_path) = adaptive_reuse.origin_public_fast_path {
            config.adaptive_reuse.origin_public_fast_path = origin_public_fast_path;
        }
        if let Some(precision) = adaptive_reuse.precision {
            config.adaptive_reuse.precision = precision;
        }
    }
    Ok(())
}
