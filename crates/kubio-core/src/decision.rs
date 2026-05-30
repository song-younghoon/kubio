use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::time::Duration;

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
    AdaptiveReuseDisabled,
    KeyValidated,
    PublicObjectValidated,
    OriginPublicCacheControl,
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
            Self::AdaptiveReuseDisabled => "Adaptive reuse is disabled by configuration.",
            Self::KeyValidated => "This cache key passed repeat-response validation.",
            Self::PublicObjectValidated => "The route looks like a public object collection.",
            Self::OriginPublicCacheControl => {
                "The origin explicitly marked this response reusable by shared caches."
            }
            Self::DiskStoreUnavailable => "The disk store was unavailable.",
            Self::DiskStoreCorruptEntry => "A corrupt disk cache entry was skipped.",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReuseClass {
    #[default]
    Watching,
    HardProtected,
    KeyValidated,
    PublicObjectCandidate,
    PublicObject,
    OriginPublic,
}

impl Display for ReuseClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Watching => f.write_str("watching"),
            Self::HardProtected => f.write_str("hard_protected"),
            Self::KeyValidated => f.write_str("key_validated"),
            Self::PublicObjectCandidate => f.write_str("public_object_candidate"),
            Self::PublicObject => f.write_str("public_object"),
            Self::OriginPublic => f.write_str("origin_public"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptiveReuseBlocker {
    Disabled,
    UnsafeRequest,
    ProtectedRoute,
    InsufficientKeyObservations,
    InsufficientShadowMatches,
    ShadowMismatch,
    InsufficientRouteSamples,
    InsufficientDistinctKeys,
    LowStoreSafeRate,
    LowPathCardinality,
    NoOriginPublicSignal,
}

impl Display for AdaptiveReuseBlocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => f.write_str("disabled"),
            Self::UnsafeRequest => f.write_str("unsafe_request"),
            Self::ProtectedRoute => f.write_str("protected_route"),
            Self::InsufficientKeyObservations => f.write_str("insufficient_key_observations"),
            Self::InsufficientShadowMatches => f.write_str("insufficient_shadow_matches"),
            Self::ShadowMismatch => f.write_str("shadow_mismatch"),
            Self::InsufficientRouteSamples => f.write_str("insufficient_route_samples"),
            Self::InsufficientDistinctKeys => f.write_str("insufficient_distinct_keys"),
            Self::LowStoreSafeRate => f.write_str("low_store_safe_rate"),
            Self::LowPathCardinality => f.write_str("low_path_cardinality"),
            Self::NoOriginPublicSignal => f.write_str("no_origin_public_signal"),
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
