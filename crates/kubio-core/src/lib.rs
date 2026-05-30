//! Shared types and deterministic helpers used across kubio.

use http::{HeaderMap, Method};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use url::form_urlencoded;
use url::Url;

pub const DEFAULT_PROXY_LISTEN: &str = "0.0.0.0:8080";
pub const DEFAULT_DASHBOARD_LISTEN: &str = "127.0.0.1:9900";
pub const DEFAULT_ORIGIN_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Watch,
    Shadow,
    Auto,
}

impl Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Watch => f.write_str("watch"),
            Self::Shadow => f.write_str("shadow"),
            Self::Auto => f.write_str("auto"),
        }
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "watch" => Ok(Self::Watch),
            "shadow" => Ok(Self::Shadow),
            "auto" => Ok(Self::Auto),
            other => Err(format!("unsupported mode `{other}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FreshnessProfile {
    Strict,
    #[default]
    Balanced,
    Relaxed,
}

impl Display for FreshnessProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Strict => f.write_str("strict"),
            Self::Balanced => f.write_str("balanced"),
            Self::Relaxed => f.write_str("relaxed"),
        }
    }
}

impl FromStr for FreshnessProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "strict" => Ok(Self::Strict),
            "balanced" => Ok(Self::Balanced),
            "relaxed" => Ok(Self::Relaxed),
            other => Err(format!("unsupported freshness profile `{other}`")),
        }
    }
}

impl FreshnessProfile {
    pub fn ttl(self) -> Duration {
        match self {
            Self::Strict => Duration::from_secs(5),
            Self::Balanced => Duration::from_secs(30),
            Self::Relaxed => Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpProtocol {
    #[default]
    Http1,
    Http2,
    Http3,
}

impl Display for HttpProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http1 => f.write_str("http1"),
            Self::Http2 => f.write_str("http2"),
            Self::Http3 => f.write_str("http3"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginProtocolPreference {
    #[default]
    Auto,
    Http1,
    Http2,
    Http3,
}

impl Display for OriginProtocolPreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Http1 => f.write_str("http1"),
            Self::Http2 => f.write_str("http2"),
            Self::Http3 => f.write_str("http3"),
        }
    }
}

impl FromStr for OriginProtocolPreference {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "http1" | "h1" | "http/1.1" => Ok(Self::Http1),
            "http2" | "h2" | "http/2" => Ok(Self::Http2),
            "http3" | "h3" | "http/3" => Ok(Self::Http3),
            other => Err(format!("unsupported origin protocol preference `{other}`")),
        }
    }
}

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
    pub alt_svc_ma: Duration,
    pub max_concurrent_streams: u64,
    pub max_field_section_size: u64,
    pub idle_timeout: Duration,
}

impl Default for Http3ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: None,
            advertise: false,
            alt_svc_ma: Duration::from_secs(3600),
            max_concurrent_streams: 128,
            max_field_section_size: 64 * 1024,
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
}

impl Default for OriginProtocolConfig {
    fn default() -> Self {
        Self {
            preferred: OriginProtocolPreference::Auto,
            fallback: true,
            http2_prior_knowledge: false,
            http3_experimental: false,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RouteId {
    pub method: String,
    pub template: String,
}

impl RouteId {
    pub fn new(method: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            template: template.into(),
        }
    }

    pub fn from_method_path(method: &Method, path: &str) -> Self {
        Self::new(method.as_str(), normalize_path_template(path))
    }

    pub fn as_label(&self) -> String {
        format!("{} {}", self.method, self.template)
    }

    pub fn hash(&self) -> String {
        short_hash(&self.as_label())
    }
}

impl Display for RouteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.template)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheKeyHash(pub String);

impl Display for CacheKeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub method: String,
    pub scheme: String,
    pub authority: String,
    pub path: String,
    pub normalized_query: String,
    pub vary_headers: Vec<(String, String)>,
}

impl CacheKey {
    pub fn hash(&self) -> CacheKeyHash {
        let mut material = String::new();
        material.push_str(&self.method);
        material.push('\n');
        material.push_str(&self.scheme);
        material.push('\n');
        material.push_str(&self.authority);
        material.push('\n');
        material.push_str(&self.path);
        material.push('\n');
        material.push_str(&self.normalized_query);
        material.push('\n');
        for (name, value) in &self.vary_headers {
            material.push_str(name);
            material.push('=');
            material.push_str(value);
            material.push('\n');
        }
        CacheKeyHash(short_hash(&material))
    }
}

pub fn build_cache_key(
    method: &Method,
    scheme: &str,
    authority: &str,
    path: &str,
    query: Option<&str>,
    request_headers: &HeaderMap,
    vary_names: &[&str],
) -> CacheKey {
    build_cache_key_with_query_config(
        method,
        scheme,
        authority,
        path,
        query,
        request_headers,
        vary_names,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_cache_key_with_query_config(
    method: &Method,
    scheme: &str,
    authority: &str,
    path: &str,
    query: Option<&str>,
    request_headers: &HeaderMap,
    vary_names: &[&str],
    query_config: Option<&RouteQueryConfig>,
) -> CacheKey {
    let mut vary_headers = vary_names
        .iter()
        .map(|name| {
            let value = request_headers
                .get(*name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_string();
            (name.to_ascii_lowercase(), value)
        })
        .collect::<Vec<_>>();
    vary_headers.sort_by(|left, right| left.0.cmp(&right.0));

    CacheKey {
        method: method.as_str().to_string(),
        scheme: scheme.to_string(),
        authority: authority.to_string(),
        path: path.to_string(),
        normalized_query: query
            .map(|query| normalize_query_with_config(query, query_config))
            .unwrap_or_default(),
        vary_headers,
    }
}

pub fn matching_route_hint<'a>(
    route_id: &RouteId,
    hints: &'a [RouteHintConfig],
) -> Option<&'a RouteHintConfig> {
    hints.iter().find(|hint| hint.matches(route_id))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseFingerprint {
    pub status: u16,
    pub header_hash: String,
    pub body_hash: Option<String>,
}

impl ResponseFingerprint {
    pub fn new(status: u16, header_hash: String, body_hash: Option<String>) -> Self {
        Self {
            status,
            header_hash,
            body_hash,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Reuse,
    StoreOnly,
    ObserveOnly,
    Protect,
    Bypass,
}

impl Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reuse => f.write_str("reuse"),
            Self::StoreOnly => f.write_str("store_only"),
            Self::ObserveOnly => f.write_str("observe_only"),
            Self::Protect => f.write_str("protect"),
            Self::Bypass => f.write_str("bypass"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    MethodNotCacheable,
    HasAuthorization,
    HasCookie,
    HasSetCookie,
    CacheControlNoStore,
    CacheControlPrivate,
    CacheControlNoCache,
    VaryUnsupported,
    VaryWildcard,
    SensitivePath,
    RangeRequest,
    RequestBodyOnGet,
    StatusNotCacheable,
    ShadowMismatch,
    InsufficientSamples,
    InsufficientShadowValidations,
    LowEstimatedBenefit,
    ObjectTooLarge,
    HeaderListTooLarge,
    FingerprintUnavailable,
    PanicSwitchActive,
    PolicyError,
    StoreError,
    ReusableAndFresh,
    ConditionalRevalidationRequired,
    RevalidationNotModified,
    RevalidationModified,
    RevalidationFailed,
    NoValidatorAvailable,
    NoCacheRequiresRevalidation,
    StaleIfErrorAllowed,
    StaleIfErrorNotAllowed,
    StaleTooOld,
    RouteHintApplied,
    RouteHintRejected,
    QueryHintApplied,
    QueryHintRejected,
    DiskStoreUnavailable,
    DiskStoreCorruptEntry,
}

impl DecisionReason {
    pub fn user_message(self) -> &'static str {
        match self {
            Self::MethodNotCacheable => "The request method is not eligible for reuse.",
            Self::HasAuthorization => "Authorization header was observed.",
            Self::HasCookie => "Cookie header was observed.",
            Self::HasSetCookie => "The origin response sets cookies.",
            Self::CacheControlNoStore => "The origin response says it must not be stored.",
            Self::CacheControlPrivate => "The origin response is marked private.",
            Self::CacheControlNoCache => {
                "The origin response requires revalidation, which v0.1.0 does not reuse."
            }
            Self::VaryUnsupported => "The response varies on headers kubio does not support yet.",
            Self::VaryWildcard => "The response uses Vary: *.",
            Self::SensitivePath => "The route looks user-specific or sensitive.",
            Self::RangeRequest => "Range requests are passed through in v0.1.0.",
            Self::RequestBodyOnGet => "GET/HEAD requests with bodies are passed through.",
            Self::StatusNotCacheable => "Only 200 responses are eligible for automatic reuse.",
            Self::ShadowMismatch => "A shadow validation saw a different response pattern.",
            Self::InsufficientSamples => "More traffic samples are required before reuse.",
            Self::InsufficientShadowValidations => {
                "More shadow validations are required before reuse."
            }
            Self::LowEstimatedBenefit => "kubio has not seen enough repeat traffic yet.",
            Self::ObjectTooLarge => "The response is larger than the configured object limit.",
            Self::HeaderListTooLarge => "The request headers exceed the configured protocol limit.",
            Self::FingerprintUnavailable => "kubio could not build a safe response fingerprint.",
            Self::PanicSwitchActive => "The panic switch is active.",
            Self::PolicyError => "A policy error caused kubio to pass through to origin.",
            Self::StoreError => "A cache store error caused kubio to pass through to origin.",
            Self::ReusableAndFresh => "A verified fresh response was available.",
            Self::ConditionalRevalidationRequired => {
                "The cached response is stale and requires origin revalidation."
            }
            Self::RevalidationNotModified => {
                "The origin confirmed the stored response is still current."
            }
            Self::RevalidationModified => "The origin returned new content during revalidation.",
            Self::RevalidationFailed => "Revalidation failed, so kubio used the safe fallback.",
            Self::NoValidatorAvailable => {
                "The cached response does not have an ETag or Last-Modified validator."
            }
            Self::NoCacheRequiresRevalidation => {
                "The origin allows storage but requires revalidation before reuse."
            }
            Self::StaleIfErrorAllowed => {
                "A verified stale response was allowed during an origin error."
            }
            Self::StaleIfErrorNotAllowed => "Stale reuse is not allowed for this route.",
            Self::StaleTooOld => "The stored response is older than the allowed stale window.",
            Self::RouteHintApplied => "A route policy hint was applied.",
            Self::RouteHintRejected => "A route policy hint was rejected by a safety rule.",
            Self::QueryHintApplied => "A configured query key hint was applied.",
            Self::QueryHintRejected => "A configured query key hint was rejected.",
            Self::DiskStoreUnavailable => "The disk store was unavailable.",
            Self::DiskStoreCorruptEntry => "A corrupt disk cache entry was skipped.",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validators {
    pub fn available(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCacheControl {
    pub max_age: Option<Duration>,
    pub stale_if_error: Option<Duration>,
    pub no_cache: bool,
    pub must_revalidate: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteState {
    #[default]
    Watching,
    Candidate,
    ShadowValidated,
    Auto,
    Protected,
}

impl Display for RouteState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Watching => f.write_str("watching"),
            Self::Candidate => f.write_str("candidate"),
            Self::ShadowValidated => f.write_str("shadow_validated"),
            Self::Auto => f.write_str("auto"),
            Self::Protected => f.write_str("protected"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusClass {
    Informational,
    Success,
    Redirection,
    ClientError,
    ServerError,
    Unknown,
}

impl StatusClass {
    pub fn from_status(status: u16) -> Self {
        match status {
            100..=199 => Self::Informational,
            200..=299 => Self::Success,
            300..=399 => Self::Redirection,
            400..=499 => Self::ClientError,
            500..=599 => Self::ServerError,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Informational => "1xx",
            Self::Success => "2xx",
            Self::Redirection => "3xx",
            Self::ClientError => "4xx",
            Self::ServerError => "5xx",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusClassCounts {
    pub informational: u64,
    pub success: u64,
    pub redirection: u64,
    pub client_error: u64,
    pub server_error: u64,
    pub unknown: u64,
}

impl StatusClassCounts {
    pub fn increment(&mut self, class: StatusClass) {
        match class {
            StatusClass::Informational => self.informational += 1,
            StatusClass::Success => self.success += 1,
            StatusClass::Redirection => self.redirection += 1,
            StatusClass::ClientError => self.client_error += 1,
            StatusClass::ServerError => self.server_error += 1,
            StatusClass::Unknown => self.unknown += 1,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencyBucketSnapshot {
    pub le_seconds: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencySnapshot {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub avg_ms: f64,
    pub count: u64,
    pub sum_ms: f64,
    pub buckets: Vec<LatencyBucketSnapshot>,
}

pub fn normalize_path_template(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }

    let mut segments = Vec::new();
    for segment in path.trim_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        let decoded = percent_decode_str(segment)
            .decode_utf8()
            .map(|value| value.to_string())
            .unwrap_or_else(|_| segment.to_string());
        if is_id_like_segment(&decoded) {
            segments.push("{id}".to_string());
        } else {
            segments.push(segment.to_string());
        }
    }

    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

pub fn normalize_query(query: &str) -> String {
    normalize_query_with_config(query, None)
}

pub fn normalize_query_with_config(query: &str, query_config: Option<&RouteQueryConfig>) -> String {
    if query.is_empty() {
        return String::new();
    }

    let mut pairs = form_urlencoded::parse(query.as_bytes())
        .enumerate()
        .map(|(index, (name, value))| (index, name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();

    if let Some(config) = query_config {
        pairs.retain(|(_, name, _)| query_param_allowed(name, config));
    }

    pairs.sort_by(|left, right| match left.1.cmp(&right.1) {
        std::cmp::Ordering::Equal => left.0.cmp(&right.0),
        ordering => ordering,
    });

    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (_, name, value) in pairs {
        serializer.append_pair(&name, &value);
    }
    serializer.finish()
}

fn query_param_allowed(name: &str, config: &RouteQueryConfig) -> bool {
    if config
        .ignore
        .iter()
        .any(|pattern| query_pattern_matches(pattern, name))
    {
        return false;
    }
    if !config.include.is_empty() {
        return config
            .include
            .iter()
            .any(|pattern| query_pattern_matches(pattern, name));
    }
    true
}

pub fn query_pattern_matches(pattern: &str, name: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        pattern == name
    }
}

pub fn is_sensitive_query_param(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "token"
            | "access_token"
            | "auth"
            | "authorization"
            | "session"
            | "password"
            | "secret"
            | "key"
            | "signature"
            | "sig"
    )
}

pub fn is_id_like_segment(segment: &str) -> bool {
    if segment.chars().all(|ch| ch.is_ascii_digit()) {
        return !segment.is_empty();
    }
    if is_uuid_like(segment) {
        return true;
    }
    if is_ulid_like(segment) {
        return true;
    }
    segment.len() >= 16 && segment.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_uuid_like(segment: &str) -> bool {
    let parts = segment.split('-').collect::<Vec<_>>();
    let lengths = [8, 4, 4, 4, 12];
    parts.len() == lengths.len()
        && parts
            .iter()
            .zip(lengths)
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

fn is_ulid_like(segment: &str) -> bool {
    segment.len() == 26
        && segment
            .chars()
            .all(|ch| matches!(ch, '0'..='9' | 'A'..='H' | 'J'..='K' | 'M'..='N' | 'P'..='T' | 'V'..='Z'))
}

pub fn sensitive_path_score(path: &str) -> u8 {
    let keywords = [
        "me", "user", "users", "account", "profile", "session", "login", "logout", "billing",
        "payment", "checkout", "admin", "token", "oauth",
    ];

    path.trim_matches('/')
        .split('/')
        .filter_map(|segment| {
            percent_decode_str(segment)
                .decode_utf8()
                .ok()
                .map(|decoded| decoded.to_ascii_lowercase())
        })
        .filter(|segment| keywords.iter().any(|keyword| segment == keyword))
        .count()
        .min(u8::MAX as usize) as u8
}

pub fn stable_header_hash(headers: &HeaderMap) -> String {
    let mut stable = headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str().to_ascii_lowercase();
            if is_volatile_header(&name) {
                return None;
            }
            value
                .to_str()
                .ok()
                .map(|value| (name, value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    stable.sort();

    let mut material = String::new();
    for (name, value) in stable {
        material.push_str(&name);
        material.push(':');
        material.push_str(&value);
        material.push('\n');
    }
    short_hash(&material)
}

pub fn body_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn short_hash(value: &str) -> String {
    let digest = blake3::hash(value.as_bytes()).to_hex().to_string();
    digest[..16].to_string()
}

pub fn is_volatile_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "date"
            | "age"
            | "server"
            | "via"
            | "x-request-id"
            | "traceparent"
            | "tracestate"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

pub fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

pub fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
    )
}

pub fn redact_header_value(name: &str, value: &str) -> String {
    if is_sensitive_header(name) {
        "REDACTED".to_string()
    } else {
        value.to_string()
    }
}

pub fn parse_size(value: &str) -> Result<u64, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("size cannot be empty".to_string());
    }

    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let number = trimmed[..split_at]
        .parse::<u64>()
        .map_err(|_| format!("invalid size `{value}`"))?;
    let unit = trimmed[split_at..].trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        _ => return Err(format!("unsupported size unit `{unit}`")),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| format!("size `{value}` is too large"))
}

pub fn parse_duration(value: &str) -> Result<Duration, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("duration cannot be empty".to_string());
    }
    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let number = trimmed[..split_at]
        .parse::<u64>()
        .map_err(|_| format!("invalid duration `{value}`"))?;
    let unit = trimmed[split_at..].trim().to_ascii_lowercase();
    match unit.as_str() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => Ok(Duration::from_secs(number)),
        "ms" | "millisecond" | "milliseconds" => Ok(Duration::from_millis(number)),
        "m" | "min" | "mins" | "minute" | "minutes" => Ok(Duration::from_secs(number * 60)),
        "h" | "hr" | "hrs" | "hour" | "hours" => Ok(Duration::from_secs(number * 60 * 60)),
        _ => Err(format!("unsupported duration unit `{unit}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn route_clustering_replaces_ids() {
        assert_eq!(
            normalize_path_template("/api/products/123"),
            "/api/products/{id}"
        );
        assert_eq!(
            normalize_path_template("/api/users/018f4df0-3e42-7046-9d81-a061d74a4c18"),
            "/api/users/{id}"
        );
        assert_eq!(
            normalize_path_template("/api/search?q=phone"),
            "/api/search"
        );
    }

    #[test]
    fn query_normalization_sorts_names_and_preserves_repeats() {
        assert_eq!(normalize_query("b=2&a=1"), "a=1&b=2");
        assert_eq!(normalize_query("b=1&a=0&b=2"), "a=0&b=1&b=2");
    }

    #[test]
    fn query_normalization_applies_route_query_config() {
        let config = RouteQueryConfig {
            include: Vec::new(),
            ignore: vec!["utm_*".to_string(), "gclid".to_string()],
        };

        assert_eq!(
            normalize_query_with_config("b=2&utm_source=x&a=1&gclid=y", Some(&config)),
            "a=1&b=2"
        );
    }

    #[test]
    fn volatile_headers_are_excluded_from_hash() {
        let mut first = HeaderMap::new();
        first.insert("date", "today".parse().unwrap());
        first.insert("content-type", "application/json".parse().unwrap());

        let mut second = HeaderMap::new();
        second.insert("date", "tomorrow".parse().unwrap());
        second.insert("content-type", "application/json".parse().unwrap());

        assert_eq!(stable_header_hash(&first), stable_header_hash(&second));
    }

    #[test]
    fn size_parser_supports_binary_units() {
        assert_eq!(parse_size("1MiB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("256 kb").unwrap(), 256 * 1024);
    }

    #[test]
    fn duration_parser_supports_common_units() {
        assert_eq!(parse_duration("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
    }

    proptest! {
        #[test]
        fn route_clustering_never_panics(path in "\\PC*") {
            let _ = normalize_path_template(&path);
        }

        #[test]
        fn query_normalization_is_stable_for_parameter_order(a in "[A-Za-z0-9]{0,16}", b in "[A-Za-z0-9]{0,16}") {
            let left = format!("b={b}&a={a}");
            let right = format!("a={a}&b={b}");

            prop_assert_eq!(normalize_query(&left), normalize_query(&right));
        }

        #[test]
        fn redaction_never_returns_sensitive_values(secret in "[A-Za-z0-9]{1,64}") {
            prop_assert_ne!(redact_header_value("authorization", &secret), secret.as_str());
            prop_assert_ne!(redact_header_value("cookie", &secret), secret.as_str());
            prop_assert_ne!(redact_header_value("set-cookie", &secret), secret.as_str());
        }
    }

    #[test]
    fn cache_key_hash_changes_with_vary_header_values() {
        let mut first = HeaderMap::new();
        first.insert("accept-language", "en".parse().unwrap());
        let mut second = HeaderMap::new();
        second.insert("accept-language", "ko".parse().unwrap());

        let method = Method::GET;
        let first_key = build_cache_key(
            &method,
            "http",
            "localhost:3000",
            "/api/products",
            Some("b=2&a=1"),
            &first,
            &["accept-language"],
        );
        let second_key = build_cache_key(
            &method,
            "http",
            "localhost:3000",
            "/api/products",
            Some("a=1&b=2"),
            &second,
            &["accept-language"],
        );

        assert_ne!(first_key.hash(), second_key.hash());
    }

    #[test]
    fn body_changes_alter_fingerprint_hash() {
        assert_ne!(body_hash(b"one"), body_hash(b"two"));
    }
}
