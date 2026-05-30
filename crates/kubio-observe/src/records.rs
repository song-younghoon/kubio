use kubio_core::{CacheKeyHash, Decision, DecisionReason, Mode, ResponseFingerprint, RouteId};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct QueryParamRecord {
    pub name: String,
    pub configured_action: String,
    pub value_hash: Option<String>,
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyObservation {
    pub cache_key_hash: CacheKeyHash,
    pub route_id: RouteId,
    pub seen_count: u64,
    pub first_seen_at: SystemTime,
    pub last_fingerprint: Option<ResponseFingerprint>,
    pub recent_shadow_matches: u32,
    pub recent_shadow_mismatches: u32,
    pub last_seen_at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct ObservationRecord {
    pub route_id: RouteId,
    pub cache_key_hash: Option<CacheKeyHash>,
    pub decision: Decision,
    pub reasons: Vec<DecisionReason>,
    pub status: u16,
    pub latency: Duration,
    pub origin: bool,
    pub reused: bool,
    pub protected: bool,
    pub bypass: bool,
    pub fingerprint: Option<ResponseFingerprint>,
    pub shadow_eligible: bool,
    pub score: i16,
    pub mode: Mode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ObservationOutcome {
    pub shadow_match: bool,
    pub shadow_mismatch: bool,
    pub candidate_detected: bool,
    pub promoted_to_auto: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevalidationOutcome {
    NotModified,
    Modified,
    Failed,
    Skipped,
}
