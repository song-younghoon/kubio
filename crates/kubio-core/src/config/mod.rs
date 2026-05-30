use crate::{FreshnessProfile, Mode, OriginProtocolPreference, RouteId};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use url::Url;

pub const DEFAULT_PROXY_LISTEN: &str = "0.0.0.0:8080";
pub const DEFAULT_DASHBOARD_LISTEN: &str = "127.0.0.1:9900";
pub const DEFAULT_ORIGIN_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveConfig {
    pub server: ServerConfig,
    pub origin: Url,
    pub origin_protocol: OriginProtocolConfig,
    pub mode: Mode,
    pub freshness: FreshnessProfile,
    pub dashboard: DashboardConfig,
    pub policy: PolicyConfig,
    pub storage: StorageConfig,
    pub performance: PerformanceConfig,
    pub observability: ObservabilityConfig,
    pub routes: Vec<RouteHintConfig>,
    pub debug_headers: bool,
    pub panic_file: Option<PathBuf>,
    pub admin_token: Option<String>,
}

impl EffectiveConfig {
    pub fn redacted(&self) -> RedactedConfig {
        RedactedConfig {
            server: self.server.clone(),
            origin: self.origin.to_string(),
            origin_protocol: self.origin_protocol.clone(),
            mode: self.mode,
            freshness: self.freshness,
            dashboard: self.dashboard.clone(),
            policy: self.policy.clone(),
            storage: self.storage.clone(),
            performance: self.performance.clone(),
            observability: self.observability.clone(),
            routes: self.routes.clone(),
            debug_headers: self.debug_headers,
            panic_file: self.panic_file.clone(),
            admin_token: self.admin_token.as_ref().map(|_| "REDACTED".to_string()),
        }
    }
}

impl Default for EffectiveConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                listen: DEFAULT_PROXY_LISTEN.parse().expect("valid default listen"),
                origin_timeout: Duration::from_millis(DEFAULT_ORIGIN_TIMEOUT_MS),
                tls: None,
                protocols: ServerProtocolConfig::default(),
                http2: Http2Config::default(),
                http3: Http3ServerConfig::default(),
            },
            origin: Url::parse("http://localhost:3000").expect("valid default origin"),
            origin_protocol: OriginProtocolConfig::default(),
            mode: Mode::Watch,
            freshness: FreshnessProfile::Balanced,
            dashboard: DashboardConfig {
                enabled: true,
                listen: DEFAULT_DASHBOARD_LISTEN
                    .parse()
                    .expect("valid default dashboard listen"),
                allow_public: false,
                admin_api: true,
            },
            policy: PolicyConfig::default(),
            storage: StorageConfig::default(),
            performance: PerformanceConfig::default(),
            observability: ObservabilityConfig::default(),
            routes: Vec::new(),
            debug_headers: false,
            panic_file: None,
            admin_token: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedactedConfig {
    pub server: ServerConfig,
    pub origin: String,
    pub origin_protocol: OriginProtocolConfig,
    pub mode: Mode,
    pub freshness: FreshnessProfile,
    pub dashboard: DashboardConfig,
    pub policy: PolicyConfig,
    pub storage: StorageConfig,
    pub performance: PerformanceConfig,
    pub observability: ObservabilityConfig,
    pub routes: Vec<RouteHintConfig>,
    pub debug_headers: bool,
    pub panic_file: Option<PathBuf>,
    pub admin_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub origin_timeout: Duration,
    pub tls: Option<TlsConfig>,
    pub protocols: ServerProtocolConfig,
    pub http2: Http2Config,
    pub http3: Http3ServerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerProtocolConfig {
    pub http1: bool,
    pub http2: bool,
    pub h2c: bool,
}

impl Default for ServerProtocolConfig {
    fn default() -> Self {
        Self {
            http1: true,
            http2: false,
            h2c: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Http2Config {
    pub max_concurrent_streams: u32,
    pub initial_stream_window_size: u32,
    pub initial_connection_window_size: u32,
    pub keepalive_interval: Option<Duration>,
    pub keepalive_timeout: Duration,
    pub max_header_list_size: u64,
}

impl Default for Http2Config {
    fn default() -> Self {
        Self {
            max_concurrent_streams: 256,
            initial_stream_window_size: mib(1) as u32,
            initial_connection_window_size: (4 * mib(1)) as u32,
            keepalive_interval: None,
            keepalive_timeout: Duration::from_secs(10),
            max_header_list_size: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Http3ServerConfig {
    pub enabled: bool,
    pub listen: Option<SocketAddr>,
    pub advertise: bool,
    pub authorities: Vec<String>,
    pub alt_svc_ma: Duration,
    pub max_concurrent_streams: u64,
    pub max_field_section_size: u64,
    pub qpack_max_table_capacity: u64,
    pub max_udp_payload_size: u16,
    pub idle_timeout: Duration,
}

impl Default for Http3ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: None,
            advertise: false,
            authorities: Vec::new(),
            alt_svc_ma: Duration::from_secs(3600),
            max_concurrent_streams: 128,
            max_field_section_size: 64 * 1024,
            qpack_max_table_capacity: 0,
            max_udp_payload_size: 1350,
            idle_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginProtocolConfig {
    pub preferred: OriginProtocolPreference,
    pub fallback: bool,
    pub http2_prior_knowledge: bool,
    pub http3_experimental: bool,
    pub http3_max_idle_connections: usize,
    pub http3_idle_timeout: Duration,
    pub http3_ca_certs: Vec<PathBuf>,
}

impl Default for OriginProtocolConfig {
    fn default() -> Self {
        Self {
            preferred: OriginProtocolPreference::Auto,
            fallback: true,
            http2_prior_knowledge: false,
            http3_experimental: false,
            http3_max_idle_connections: 32,
            http3_idle_timeout: Duration::from_secs(90),
            http3_ca_certs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardConfig {
    pub enabled: bool,
    pub listen: SocketAddr,
    pub allow_public: bool,
    pub admin_api: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyConfig {
    pub respect_origin_headers: bool,
    pub protect_authorization: bool,
    pub protect_cookies: bool,
    pub protect_set_cookie: bool,
    pub max_object_size: u64,
    pub max_fingerprint_body_size: u64,
    pub max_request_body_size: usize,
    pub min_route_samples: u64,
    pub min_key_repeats: u64,
    pub min_shadow_validations: u64,
    pub max_shadow_mismatch_rate: f64,
    pub revalidation: RevalidationConfig,
    pub stale_if_error: StaleIfErrorConfig,
    pub query_intelligence: QueryIntelligenceConfig,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            respect_origin_headers: true,
            protect_authorization: true,
            protect_cookies: true,
            protect_set_cookie: true,
            max_object_size: mib(1),
            max_fingerprint_body_size: mib(2),
            max_request_body_size: 16 * 1024 * 1024,
            min_route_samples: 100,
            min_key_repeats: 5,
            min_shadow_validations: 20,
            max_shadow_mismatch_rate: 0.001,
            revalidation: RevalidationConfig::default(),
            stale_if_error: StaleIfErrorConfig::default(),
            query_intelligence: QueryIntelligenceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevalidationConfig {
    pub enabled: bool,
    pub prefer_etag: bool,
    pub max_validator_length: usize,
}

impl Default for RevalidationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prefer_etag: true,
            max_validator_length: 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleIfErrorMode {
    Disabled,
    #[default]
    Origin,
    Enabled,
}

impl Display for StaleIfErrorMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => f.write_str("disabled"),
            Self::Origin => f.write_str("origin"),
            Self::Enabled => f.write_str("enabled"),
        }
    }
}

impl FromStr for StaleIfErrorMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "disabled" => Ok(Self::Disabled),
            "origin" => Ok(Self::Origin),
            "enabled" => Ok(Self::Enabled),
            other => Err(format!("unsupported stale-if-error mode `{other}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleIfErrorConfig {
    pub mode: StaleIfErrorMode,
    pub max_stale: Duration,
}

impl Default for StaleIfErrorConfig {
    fn default() -> Self {
        Self {
            mode: StaleIfErrorMode::Origin,
            max_stale: Duration::from_secs(300),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryIntelligenceConfig {
    pub enabled: bool,
    pub auto_ignore: bool,
}

impl Default for QueryIntelligenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_ignore: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageConfig {
    pub kind: String,
    pub max_size: u64,
    pub max_object_size: u64,
    pub path: Option<PathBuf>,
    pub sync: bool,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            kind: "memory".to_string(),
            max_size: mib(256),
            max_object_size: mib(1),
            path: None,
            sync: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceConfig {
    pub max_in_flight_requests: usize,
    pub max_buffered_response_size: u64,
    pub stream_unstoreable_bodies: bool,
    pub observer_shards: usize,
    pub async_disk_writes: bool,
    pub origin_pool_max_idle_per_host: usize,
    pub origin_pool_idle_timeout: Duration,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            max_in_flight_requests: 4096,
            max_buffered_response_size: mib(2),
            stream_unstoreable_bodies: true,
            observer_shards: 64,
            async_disk_writes: true,
            origin_pool_max_idle_per_host: 32,
            origin_pool_idle_timeout: Duration::from_secs(90),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteHintConfig {
    pub name: Option<String>,
    #[serde(rename = "match")]
    pub route_match: RouteMatchConfig,
    #[serde(default)]
    pub freshness: RouteFreshnessConfig,
    #[serde(default)]
    pub query: RouteQueryConfig,
    #[serde(default)]
    pub vary: RouteVaryConfig,
    #[serde(default)]
    pub stale_if_error: RouteStaleIfErrorConfig,
    #[serde(default)]
    pub safety: RouteSafetyConfig,
}

impl RouteHintConfig {
    pub fn matches(&self, route_id: &RouteId) -> bool {
        self.route_match
            .method
            .eq_ignore_ascii_case(&route_id.method)
            && self.route_match.path == route_id.template
    }

    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("{} {}", self.route_match.method, self.route_match.path))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteMatchConfig {
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteFreshnessConfig {
    pub ttl: Option<Duration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteQueryConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
}

impl RouteQueryConfig {
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.ignore.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteVaryConfig {
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteStaleIfErrorConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub max_stale: Option<Duration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSafetyConfig {
    #[serde(default)]
    pub acknowledge_sensitive_path: bool,
    #[serde(default)]
    pub force_protect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub metrics: bool,
    pub metrics_path: String,
    pub tracing: bool,
    pub max_routes: usize,
    pub max_keys: usize,
    pub max_events: usize,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            metrics: true,
            metrics_path: "/metrics".to_string(),
            tracing: true,
            max_routes: 10_000,
            max_keys: 100_000,
            max_events: 1_000,
        }
    }
}

pub const fn mib(value: u64) -> u64 {
    value * 1024 * 1024
}
