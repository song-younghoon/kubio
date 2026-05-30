use anyhow::{bail, Result};
use kubio_core::{
    parse_duration, RouteFreshnessConfig, RouteHintConfig, RouteMatchConfig, RouteQueryConfig,
    RouteSafetyConfig, RouteStaleIfErrorConfig, RouteVaryConfig,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileConfig {
    #[allow(dead_code)]
    pub(crate) version: Option<u64>,
    pub(crate) server: Option<FileServerConfig>,
    pub(crate) origin: Option<String>,
    pub(crate) origin_protocol: Option<FileOriginProtocolConfig>,
    pub(crate) mode: Option<String>,
    pub(crate) freshness: Option<String>,
    pub(crate) dashboard: Option<FileDashboardConfig>,
    pub(crate) policy: Option<FilePolicyConfig>,
    pub(crate) storage: Option<FileStorageConfig>,
    pub(crate) performance: Option<FilePerformanceConfig>,
    pub(crate) observability: Option<FileObservabilityConfig>,
    pub(crate) routes: Option<Vec<FileRouteHintConfig>>,
    pub(crate) debug_headers: Option<bool>,
    pub(crate) panic_file: Option<String>,
    pub(crate) admin_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileServerConfig {
    pub(crate) listen: Option<String>,
    pub(crate) origin_timeout_ms: Option<u64>,
    pub(crate) tls: Option<FileTlsConfig>,
    pub(crate) protocols: Option<FileServerProtocolConfig>,
    pub(crate) http2: Option<FileHttp2Config>,
    pub(crate) http3: Option<FileHttp3Config>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileTlsConfig {
    pub(crate) cert: String,
    pub(crate) key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileServerProtocolConfig {
    pub(crate) http1: Option<bool>,
    pub(crate) http2: Option<bool>,
    pub(crate) h2c: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileHttp2Config {
    pub(crate) max_concurrent_streams: Option<u32>,
    pub(crate) initial_stream_window_size: Option<String>,
    pub(crate) initial_connection_window_size: Option<String>,
    pub(crate) keepalive_interval: Option<String>,
    pub(crate) keepalive_timeout: Option<String>,
    pub(crate) max_header_list_size: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileHttp3Config {
    pub(crate) enabled: Option<bool>,
    pub(crate) listen: Option<String>,
    pub(crate) advertise: Option<bool>,
    pub(crate) authorities: Option<Vec<String>>,
    pub(crate) alt_svc_ma: Option<String>,
    pub(crate) max_concurrent_streams: Option<u64>,
    pub(crate) max_field_section_size: Option<String>,
    pub(crate) qpack_max_table_capacity: Option<String>,
    pub(crate) max_udp_payload_size: Option<String>,
    pub(crate) idle_timeout: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileOriginProtocolConfig {
    pub(crate) preferred: Option<String>,
    pub(crate) fallback: Option<bool>,
    pub(crate) http2_prior_knowledge: Option<bool>,
    pub(crate) http3_experimental: Option<bool>,
    pub(crate) http3_max_idle_connections: Option<usize>,
    pub(crate) http3_idle_timeout: Option<String>,
    pub(crate) http3_ca_certs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileDashboardConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) listen: Option<String>,
    pub(crate) allow_public: Option<bool>,
    pub(crate) admin_api: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FilePolicyConfig {
    pub(crate) respect_origin_headers: Option<bool>,
    pub(crate) protect_authorization: Option<bool>,
    pub(crate) protect_cookies: Option<bool>,
    pub(crate) protect_set_cookie: Option<bool>,
    pub(crate) max_object_size: Option<String>,
    pub(crate) max_fingerprint_body_size: Option<String>,
    pub(crate) min_route_samples: Option<u64>,
    pub(crate) min_key_repeats: Option<u64>,
    pub(crate) min_shadow_validations: Option<u64>,
    pub(crate) max_shadow_mismatch_rate: Option<f64>,
    pub(crate) revalidation: Option<FileRevalidationConfig>,
    pub(crate) stale_if_error: Option<FileStaleIfErrorConfig>,
    pub(crate) query_intelligence: Option<FileQueryIntelligenceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileRevalidationConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) prefer_etag: Option<bool>,
    pub(crate) max_validator_length: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileStaleIfErrorConfig {
    pub(crate) mode: Option<String>,
    pub(crate) max_stale: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileQueryIntelligenceConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) auto_ignore: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileStorageConfig {
    pub(crate) kind: Option<String>,
    pub(crate) max_size: Option<String>,
    pub(crate) max_object_size: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) sync: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FilePerformanceConfig {
    pub(crate) max_in_flight_requests: Option<usize>,
    pub(crate) max_buffered_response_size: Option<String>,
    pub(crate) stream_unstoreable_bodies: Option<bool>,
    pub(crate) observer_shards: Option<usize>,
    pub(crate) async_disk_writes: Option<bool>,
    pub(crate) origin_pool_max_idle_per_host: Option<usize>,
    pub(crate) origin_pool_idle_timeout: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileObservabilityConfig {
    pub(crate) metrics: Option<bool>,
    pub(crate) metrics_path: Option<String>,
    pub(crate) tracing: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileRouteHintConfig {
    pub(crate) name: Option<String>,
    #[serde(rename = "match")]
    pub(crate) route_match: FileRouteMatchConfig,
    pub(crate) freshness: Option<FileRouteFreshnessConfig>,
    pub(crate) query: Option<RouteQueryConfig>,
    pub(crate) vary: Option<RouteVaryConfig>,
    pub(crate) stale_if_error: Option<FileRouteStaleIfErrorConfig>,
    pub(crate) safety: Option<RouteSafetyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileRouteMatchConfig {
    pub(crate) method: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileRouteFreshnessConfig {
    pub(crate) ttl: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileRouteStaleIfErrorConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) max_stale: Option<String>,
}

impl TryFrom<FileRouteHintConfig> for RouteHintConfig {
    type Error = anyhow::Error;

    fn try_from(value: FileRouteHintConfig) -> Result<Self> {
        let query = match value.query {
            Some(query) if query.is_empty() => {
                bail!("routes[].query must define include or ignore patterns")
            }
            Some(query) => query,
            None => RouteQueryConfig::default(),
        };
        let freshness = RouteFreshnessConfig {
            ttl: value
                .freshness
                .and_then(|freshness| freshness.ttl)
                .map(|ttl| parse_duration(&ttl).map_err(anyhow::Error::msg))
                .transpose()?,
        };
        let stale_if_error = match value.stale_if_error {
            Some(stale) => RouteStaleIfErrorConfig {
                enabled: stale.enabled.unwrap_or(false),
                max_stale: stale
                    .max_stale
                    .map(|duration| parse_duration(&duration).map_err(anyhow::Error::msg))
                    .transpose()?,
            },
            None => RouteStaleIfErrorConfig::default(),
        };
        Ok(RouteHintConfig {
            name: value.name,
            route_match: RouteMatchConfig {
                method: value.route_match.method,
                path: value.route_match.path,
            },
            freshness,
            query,
            vary: value.vary.unwrap_or_default(),
            stale_if_error,
            safety: value.safety.unwrap_or_default(),
        })
    }
}
