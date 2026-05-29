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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveConfig {
    pub server: ServerConfig,
    pub origin: Url,
    pub mode: Mode,
    pub freshness: FreshnessProfile,
    pub dashboard: DashboardConfig,
    pub policy: PolicyConfig,
    pub storage: StorageConfig,
    pub observability: ObservabilityConfig,
    pub debug_headers: bool,
    pub panic_file: Option<PathBuf>,
    pub admin_token: Option<String>,
}

impl EffectiveConfig {
    pub fn redacted(&self) -> RedactedConfig {
        RedactedConfig {
            server: self.server.clone(),
            origin: self.origin.to_string(),
            mode: self.mode,
            freshness: self.freshness,
            dashboard: self.dashboard.clone(),
            policy: self.policy.clone(),
            storage: self.storage.clone(),
            observability: self.observability.clone(),
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
            },
            origin: Url::parse("http://localhost:3000").expect("valid default origin"),
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
            observability: ObservabilityConfig::default(),
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
    pub mode: Mode,
    pub freshness: FreshnessProfile,
    pub dashboard: DashboardConfig,
    pub policy: PolicyConfig,
    pub storage: StorageConfig,
    pub observability: ObservabilityConfig,
    pub debug_headers: bool,
    pub panic_file: Option<PathBuf>,
    pub admin_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub listen: SocketAddr,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageConfig {
    pub kind: String,
    pub max_size: u64,
    pub max_object_size: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            kind: "memory".to_string(),
            max_size: mib(256),
            max_object_size: mib(1),
        }
    }
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
        normalized_query: query.map(normalize_query).unwrap_or_default(),
        vary_headers,
    }
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
    FingerprintUnavailable,
    PanicSwitchActive,
    PolicyError,
    StoreError,
    ReusableAndFresh,
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
            Self::FingerprintUnavailable => "kubio could not build a safe response fingerprint.",
            Self::PanicSwitchActive => "The panic switch is active.",
            Self::PolicyError => "A policy error caused kubio to pass through to origin.",
            Self::StoreError => "A cache store error caused kubio to pass through to origin.",
            Self::ReusableAndFresh => "A verified fresh response was available.",
        }
    }
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
pub struct LatencySnapshot {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub avg_ms: f64,
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
    if query.is_empty() {
        return String::new();
    }

    let mut pairs = form_urlencoded::parse(query.as_bytes())
        .enumerate()
        .map(|(index, (name, value))| (index, name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
