use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::time::Duration;

pub const RESPONSE_HEADER_FINGERPRINT_POLICY_VERSION: u16 = 2;

pub fn default_response_header_fingerprint_policy_version() -> u16 {
    RESPONSE_HEADER_FINGERPRINT_POLICY_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseFingerprint {
    pub status: u16,
    pub header_hash: String,
    pub body_hash: Option<String>,
    #[serde(default = "default_response_header_fingerprint_policy_version")]
    pub header_policy_version: u16,
}

impl ResponseFingerprint {
    pub fn new(status: u16, header_hash: String, body_hash: Option<String>) -> Self {
        Self::new_with_policy(
            status,
            header_hash,
            body_hash,
            RESPONSE_HEADER_FINGERPRINT_POLICY_VERSION,
        )
    }

    pub fn new_with_policy(
        status: u16,
        header_hash: String,
        body_hash: Option<String>,
        header_policy_version: u16,
    ) -> Self {
        Self {
            status,
            header_hash,
            body_hash,
            header_policy_version,
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
    QueryEquivalence,
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
            Self::QueryEquivalence => f.write_str("query_equivalence"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceTier {
    #[default]
    Unknown,
    Probation,
    Validated,
    Strong,
    Cooldown,
    HardProtected,
}

impl Display for ConfidenceTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown"),
            Self::Probation => f.write_str("probation"),
            Self::Validated => f.write_str("validated"),
            Self::Strong => f.write_str("strong"),
            Self::Cooldown => f.write_str("cooldown"),
            Self::HardProtected => f.write_str("hard_protected"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryEquivalenceClass {
    #[default]
    Unknown,
    CandidateIgnore,
    VerifiedIgnoreCandidate,
    Compacted,
    SensitiveBlocked,
    MismatchCooldown,
}

impl Display for QueryEquivalenceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown"),
            Self::CandidateIgnore => f.write_str("candidate_ignore"),
            Self::VerifiedIgnoreCandidate => f.write_str("verified_ignore_candidate"),
            Self::Compacted => f.write_str("compacted"),
            Self::SensitiveBlocked => f.write_str("sensitive_blocked"),
            Self::MismatchCooldown => f.write_str("mismatch_cooldown"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeaderEquivalenceClass {
    #[default]
    Unknown,
    DefaultIgnored,
    CandidateVolatile,
    VerifiedVolatileCandidate,
    Ignored,
    SensitiveBlocked,
    MismatchCooldown,
    ForceIncluded,
}

impl Display for HeaderEquivalenceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown"),
            Self::DefaultIgnored => f.write_str("default_ignored"),
            Self::CandidateVolatile => f.write_str("candidate_volatile"),
            Self::VerifiedVolatileCandidate => f.write_str("verified_volatile_candidate"),
            Self::Ignored => f.write_str("ignored"),
            Self::SensitiveBlocked => f.write_str("sensitive_blocked"),
            Self::MismatchCooldown => f.write_str("mismatch_cooldown"),
            Self::ForceIncluded => f.write_str("force_included"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeaderEquivalenceSource {
    #[default]
    DefaultPolicy,
    RouteHint,
    VerifiedEvidence,
    GlobalConfig,
    ForceInclude,
}

impl Display for HeaderEquivalenceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultPolicy => f.write_str("default_policy"),
            Self::RouteHint => f.write_str("route_hint"),
            Self::VerifiedEvidence => f.write_str("verified_evidence"),
            Self::GlobalConfig => f.write_str("global_config"),
            Self::ForceInclude => f.write_str("force_include"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyShape {
    #[default]
    Exact,
    QueryCompacted,
}

impl Display for KeyShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact => f.write_str("exact"),
            Self::QueryCompacted => f.write_str("query_compacted"),
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
    StaleEvidence,
    CooldownActive,
    CanaryMismatch,
    SensitiveQueryParam,
    InsufficientQueryEquivalence,
    OperatorEnablementRequired,
    VariantUnbounded,
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
            Self::StaleEvidence => f.write_str("stale_evidence"),
            Self::CooldownActive => f.write_str("cooldown_active"),
            Self::CanaryMismatch => f.write_str("canary_mismatch"),
            Self::SensitiveQueryParam => f.write_str("sensitive_query_param"),
            Self::InsufficientQueryEquivalence => f.write_str("insufficient_query_equivalence"),
            Self::OperatorEnablementRequired => f.write_str("operator_enablement_required"),
            Self::VariantUnbounded => f.write_str("variant_unbounded"),
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
