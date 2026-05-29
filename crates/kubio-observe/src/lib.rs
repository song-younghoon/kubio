//! Process-local observation state for kubio.

use kubio_core::{
    CacheKeyHash, Decision, DecisionReason, LatencyBucketSnapshot, LatencySnapshot, Mode,
    ResponseFingerprint, RouteId, RouteState, StatusClass, StatusClassCounts,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, SystemTime};

#[derive(Debug)]
pub struct Observer {
    inner: Mutex<ObserverInner>,
    max_routes: usize,
    max_keys: usize,
    max_events: usize,
    min_route_samples: u64,
    min_key_repeats: u64,
    min_shadow_validations: u64,
}

impl Observer {
    pub fn new(
        max_routes: usize,
        max_keys: usize,
        max_events: usize,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
    ) -> Self {
        Self {
            inner: Mutex::new(ObserverInner::default()),
            max_routes,
            max_keys,
            max_events,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
        }
    }

    pub fn record(&self, record: ObservationRecord) -> ObservationOutcome {
        let mut inner = self.inner.lock();
        if inner.routes.len() >= self.max_routes && !inner.routes.contains_key(&record.route_id) {
            self.push_event_locked(
                &mut inner,
                EventType::RouteLimitReached,
                None,
                None,
                vec![DecisionReason::LowEstimatedBenefit],
                "route observation limit reached",
            );
            return ObservationOutcome::default();
        }

        {
            let route = inner
                .routes
                .entry(record.route_id.clone())
                .or_insert_with(|| RouteStats::new(record.route_id.clone()));

            route.request_count += 1;
            if record.origin {
                route.origin_count += 1;
            }
            if record.reused {
                route.reuse_count += 1;
            }
            if record.protected {
                route.protected_count += 1;
            }
            if record.bypass {
                route.bypass_count += 1;
            }
            route.score = record.score;
            route.reasons = record.reasons.clone();
            route
                .status_classes
                .increment(StatusClass::from_status(record.status));
            route.latencies.push_back(record.latency);
            while route.latencies.len() > 1024 {
                route.latencies.pop_front();
            }

            if record.protected && route.state != RouteState::Auto {
                route.state = RouteState::Protected;
            }
        }

        let mut outcome = ObservationOutcome::default();
        if let (Some(key_hash), Some(fingerprint)) =
            (record.cache_key_hash.clone(), record.fingerprint.clone())
        {
            if inner.keys.len() >= self.max_keys && !inner.keys.contains_key(&key_hash) {
                self.evict_oldest_key_locked(&mut inner);
            }

            let now = SystemTime::now();
            {
                let key = inner
                    .keys
                    .entry(key_hash.clone())
                    .or_insert_with(|| KeyObservation {
                        cache_key_hash: key_hash.clone(),
                        route_id: record.route_id.clone(),
                        seen_count: 0,
                        first_seen_at: now,
                        last_fingerprint: None,
                        recent_shadow_matches: 0,
                        recent_shadow_mismatches: 0,
                        last_seen_at: now,
                    });
                key.seen_count += 1;
                key.last_seen_at = now;

                if record.shadow_eligible {
                    if let Some(previous) = &key.last_fingerprint {
                        if previous == &fingerprint {
                            key.recent_shadow_matches += 1;
                            outcome.shadow_match = true;
                        } else {
                            key.recent_shadow_mismatches += 1;
                            outcome.shadow_mismatch = true;
                        }
                    }
                }
                key.last_fingerprint = Some(fingerprint);
            }

            if outcome.shadow_match || outcome.shadow_mismatch {
                if let Some(route) = inner.routes.get_mut(&record.route_id) {
                    if outcome.shadow_match {
                        route.shadow_matches += 1;
                    }
                    if outcome.shadow_mismatch {
                        route.shadow_mismatches += 1;
                        route.state = RouteState::Protected;
                        route.reasons = vec![DecisionReason::ShadowMismatch];
                    }
                }
            }

            if outcome.shadow_mismatch {
                self.push_event_locked(
                    &mut inner,
                    EventType::RouteDemotedDueToShadowMismatch,
                    Some(record.route_id.clone()),
                    Some(key_hash.clone()),
                    vec![DecisionReason::ShadowMismatch],
                    "route demoted because shadow validation found a different response pattern",
                );
            }
        }

        self.update_route_state_locked(&mut inner, &record, &mut outcome);
        self.record_policy_event_locked(&mut inner, &record);
        outcome
    }

    pub fn record_reuse(
        &self,
        route_id: RouteId,
        cache_key_hash: CacheKeyHash,
        status: u16,
        latency: Duration,
    ) {
        let record = ObservationRecord {
            route_id,
            cache_key_hash: Some(cache_key_hash),
            decision: Decision::Reuse,
            reasons: vec![DecisionReason::ReusableAndFresh],
            status,
            latency,
            origin: false,
            reused: true,
            protected: false,
            bypass: false,
            fingerprint: None,
            shadow_eligible: false,
            score: 100,
            mode: Mode::Auto,
        };
        self.record(record);
    }

    pub fn route_state(&self, route_id: &RouteId) -> RouteState {
        self.inner
            .lock()
            .routes
            .get(route_id)
            .map(|route| route.state)
            .unwrap_or(RouteState::Watching)
    }

    pub fn is_auto_eligible(&self, route_id: &RouteId, key_hash: &CacheKeyHash) -> bool {
        let inner = self.inner.lock();
        let route_ok = inner
            .routes
            .get(route_id)
            .map(|route| route.state == RouteState::Auto)
            .unwrap_or(false);
        let key_ok = inner
            .keys
            .get(key_hash)
            .map(|key| {
                key.seen_count >= self.min_key_repeats
                    && key.recent_shadow_matches as u64 >= self.min_shadow_validations
                    && key.recent_shadow_mismatches == 0
            })
            .unwrap_or(false);
        route_ok && key_ok
    }

    pub fn snapshot(&self) -> ObserverSnapshot {
        let inner = self.inner.lock();
        let mut routes = inner
            .routes
            .values()
            .map(RouteStats::snapshot)
            .collect::<Vec<_>>();
        routes.sort_by(|left, right| {
            state_sort_key(right.state)
                .cmp(&state_sort_key(left.state))
                .then_with(|| right.estimated_savings.total_cmp(&left.estimated_savings))
                .then_with(|| right.request_count.cmp(&left.request_count))
        });

        let overview = OverviewSnapshot::from_routes(&routes);
        ObserverSnapshot {
            overview,
            routes,
            events: inner.events.iter().cloned().collect(),
        }
    }

    pub fn route_by_hash(&self, route_hash: &str) -> Option<RouteSnapshot> {
        self.inner
            .lock()
            .routes
            .values()
            .find(|route| route.route_id.hash() == route_hash)
            .map(RouteStats::snapshot)
    }

    fn update_route_state_locked(
        &self,
        inner: &mut ObserverInner,
        record: &ObservationRecord,
        outcome: &mut ObservationOutcome,
    ) {
        let mut event = None;
        {
            let Some(route) = inner.routes.get_mut(&record.route_id) else {
                return;
            };

            if route.shadow_mismatches > 0 {
                route.state = RouteState::Protected;
                return;
            }
            if record.protected {
                return;
            }

            let high_repeat = route.repeat_rate() >= 0.2;
            if route.request_count >= self.min_route_samples
                && high_repeat
                && route.score >= 50
                && route.state == RouteState::Watching
            {
                route.state = RouteState::Candidate;
                outcome.candidate_detected = true;
                event = Some((
                    EventType::RouteCandidateDetected,
                    route.reasons.clone(),
                    "route has repeated safe traffic and is a reuse candidate",
                ));
            }

            if route.shadow_matches >= self.min_shadow_validations
                && route.shadow_mismatches == 0
                && route.request_count >= self.min_route_samples
            {
                if record.mode == Mode::Auto {
                    if route.state != RouteState::Auto {
                        route.state = RouteState::Auto;
                        outcome.promoted_to_auto = true;
                        event = Some((
                            EventType::RoutePromotedToAuto,
                            vec![DecisionReason::ReusableAndFresh],
                            "route promoted to automatic reuse",
                        ));
                    }
                } else if route.state != RouteState::ShadowValidated {
                    route.state = RouteState::ShadowValidated;
                    event = Some((
                        EventType::RoutePromotedToShadow,
                        vec![DecisionReason::InsufficientShadowValidations],
                        "route passed shadow validation",
                    ));
                }
            }
        }

        if let Some((event_type, reasons, message)) = event {
            self.push_event_locked(
                inner,
                event_type,
                Some(record.route_id.clone()),
                record.cache_key_hash.clone(),
                reasons,
                message,
            );
        }
    }

    fn record_policy_event_locked(&self, inner: &mut ObserverInner, record: &ObservationRecord) {
        if record.reasons.contains(&DecisionReason::HasAuthorization) {
            self.push_event_locked(
                inner,
                EventType::RequestProtectedDueToAuthorization,
                Some(record.route_id.clone()),
                record.cache_key_hash.clone(),
                record.reasons.clone(),
                "request protected because Authorization was observed",
            );
        } else if record.reasons.contains(&DecisionReason::HasCookie) {
            self.push_event_locked(
                inner,
                EventType::RequestProtectedDueToCookie,
                Some(record.route_id.clone()),
                record.cache_key_hash.clone(),
                record.reasons.clone(),
                "request protected because Cookie was observed",
            );
        } else if record
            .reasons
            .contains(&DecisionReason::CacheControlNoStore)
        {
            self.push_event_locked(
                inner,
                EventType::ResponseNotStoredDueToNoStore,
                Some(record.route_id.clone()),
                record.cache_key_hash.clone(),
                record.reasons.clone(),
                "response not stored because Cache-Control: no-store was observed",
            );
        } else if record
            .reasons
            .contains(&DecisionReason::CacheControlPrivate)
        {
            self.push_event_locked(
                inner,
                EventType::ResponseNotStoredDueToPrivate,
                Some(record.route_id.clone()),
                record.cache_key_hash.clone(),
                record.reasons.clone(),
                "response not stored because Cache-Control: private was observed",
            );
        }
    }

    fn push_event_locked(
        &self,
        inner: &mut ObserverInner,
        event_type: EventType,
        route_id: Option<RouteId>,
        cache_key_hash: Option<CacheKeyHash>,
        reasons: Vec<DecisionReason>,
        message: impl Into<String>,
    ) {
        inner.events.push_back(Event {
            timestamp: SystemTime::now(),
            event_type,
            route_id,
            cache_key_hash,
            reasons,
            message: message.into(),
        });
        while inner.events.len() > self.max_events {
            inner.events.pop_front();
        }
    }

    fn evict_oldest_key_locked(&self, inner: &mut ObserverInner) {
        if let Some(oldest) = inner
            .keys
            .iter()
            .min_by_key(|(_, key)| key.last_seen_at)
            .map(|(hash, _)| hash.clone())
        {
            inner.keys.remove(&oldest);
        }
    }

    pub fn push_event(
        &self,
        event_type: EventType,
        route_id: Option<RouteId>,
        cache_key_hash: Option<CacheKeyHash>,
        reasons: Vec<DecisionReason>,
        message: impl Into<String>,
    ) {
        let mut inner = self.inner.lock();
        self.push_event_locked(
            &mut inner,
            event_type,
            route_id,
            cache_key_hash,
            reasons,
            message,
        );
    }
}

#[derive(Debug, Default)]
struct ObserverInner {
    routes: HashMap<RouteId, RouteStats>,
    keys: HashMap<CacheKeyHash, KeyObservation>,
    events: VecDeque<Event>,
}

#[derive(Debug, Clone)]
struct RouteStats {
    route_id: RouteId,
    state: RouteState,
    request_count: u64,
    origin_count: u64,
    reuse_count: u64,
    protected_count: u64,
    bypass_count: u64,
    shadow_matches: u64,
    shadow_mismatches: u64,
    status_classes: StatusClassCounts,
    latencies: VecDeque<Duration>,
    score: i16,
    reasons: Vec<DecisionReason>,
}

impl RouteStats {
    fn new(route_id: RouteId) -> Self {
        Self {
            route_id,
            state: RouteState::Watching,
            request_count: 0,
            origin_count: 0,
            reuse_count: 0,
            protected_count: 0,
            bypass_count: 0,
            shadow_matches: 0,
            shadow_mismatches: 0,
            status_classes: StatusClassCounts::default(),
            latencies: VecDeque::new(),
            score: 0,
            reasons: Vec::new(),
        }
    }

    fn repeat_rate(&self) -> f64 {
        if self.request_count == 0 {
            0.0
        } else {
            self.shadow_matches as f64 / self.request_count as f64
        }
    }

    fn estimated_savings(&self) -> f64 {
        if self.request_count == 0 {
            0.0
        } else {
            let reusable = self
                .request_count
                .saturating_sub(self.protected_count)
                .saturating_sub(self.bypass_count);
            reusable as f64 / self.request_count as f64
        }
    }

    fn snapshot(&self) -> RouteSnapshot {
        RouteSnapshot {
            route_id: self.route_id.clone(),
            route_hash: self.route_id.hash(),
            state: self.state,
            request_count: self.request_count,
            origin_count: self.origin_count,
            reuse_count: self.reuse_count,
            protected_count: self.protected_count,
            bypass_count: self.bypass_count,
            shadow_matches: self.shadow_matches,
            shadow_mismatches: self.shadow_mismatches,
            status_classes: self.status_classes.clone(),
            latency: latency_snapshot(&self.latencies),
            repeat_rate: self.repeat_rate(),
            estimated_savings: self.estimated_savings(),
            actual_reuse_rate: if self.request_count == 0 {
                0.0
            } else {
                self.reuse_count as f64 / self.request_count as f64
            },
            score: self.score,
            reasons: self.reasons.clone(),
            explanation: self
                .reasons
                .iter()
                .map(|reason| reason.user_message().to_string())
                .collect(),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverSnapshot {
    pub overview: OverviewSnapshot,
    pub routes: Vec<RouteSnapshot>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OverviewSnapshot {
    pub observed_requests: u64,
    pub origin_requests: u64,
    pub reused_responses: u64,
    pub protected_requests: u64,
    pub bypassed_requests: u64,
    pub candidate_routes: u64,
    pub auto_routes: u64,
    pub estimated_savings: f64,
    pub actual_reuse_rate: f64,
    pub shadow_matches: u64,
    pub shadow_mismatches: u64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
}

impl OverviewSnapshot {
    fn from_routes(routes: &[RouteSnapshot]) -> Self {
        let mut overview = Self::default();
        let mut latencies = Vec::new();
        for route in routes {
            overview.observed_requests += route.request_count;
            overview.origin_requests += route.origin_count;
            overview.reused_responses += route.reuse_count;
            overview.protected_requests += route.protected_count;
            overview.bypassed_requests += route.bypass_count;
            overview.shadow_matches += route.shadow_matches;
            overview.shadow_mismatches += route.shadow_mismatches;
            if route.state == RouteState::Candidate || route.state == RouteState::ShadowValidated {
                overview.candidate_routes += 1;
            }
            if route.state == RouteState::Auto {
                overview.auto_routes += 1;
            }
            latencies.push(route.latency.p50_ms);
            latencies.push(route.latency.p95_ms);
        }
        if overview.observed_requests > 0 {
            overview.estimated_savings = routes
                .iter()
                .map(|route| route.estimated_savings * route.request_count as f64)
                .sum::<f64>()
                / overview.observed_requests as f64;
            overview.actual_reuse_rate =
                overview.reused_responses as f64 / overview.observed_requests as f64;
        }
        if !latencies.is_empty() {
            latencies.sort_by(|left, right| left.total_cmp(right));
            overview.p50_latency_ms = percentile(&latencies, 0.50);
            overview.p95_latency_ms = percentile(&latencies, 0.95);
        }
        overview
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSnapshot {
    pub route_id: RouteId,
    pub route_hash: String,
    pub state: RouteState,
    pub request_count: u64,
    pub origin_count: u64,
    pub reuse_count: u64,
    pub protected_count: u64,
    pub bypass_count: u64,
    pub shadow_matches: u64,
    pub shadow_mismatches: u64,
    pub status_classes: StatusClassCounts,
    pub latency: LatencySnapshot,
    pub repeat_rate: f64,
    pub estimated_savings: f64,
    pub actual_reuse_rate: f64,
    pub score: i16,
    pub reasons: Vec<DecisionReason>,
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub timestamp: SystemTime,
    pub event_type: EventType,
    pub route_id: Option<RouteId>,
    pub cache_key_hash: Option<CacheKeyHash>,
    pub reasons: Vec<DecisionReason>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    RouteCandidateDetected,
    RoutePromotedToShadow,
    RoutePromotedToAuto,
    RouteDemotedDueToShadowMismatch,
    RequestProtectedDueToAuthorization,
    RequestProtectedDueToCookie,
    ResponseNotStoredDueToNoStore,
    ResponseNotStoredDueToPrivate,
    CacheEntryEvicted,
    OriginRequestFailed,
    StoreErrorFailOpen,
    PanicSwitchEnabled,
    PanicSwitchDisabled,
    RouteLimitReached,
}

fn state_sort_key(state: RouteState) -> u8 {
    match state {
        RouteState::Auto => 4,
        RouteState::Candidate | RouteState::ShadowValidated => 3,
        RouteState::Protected => 2,
        RouteState::Watching => 1,
    }
}

fn latency_snapshot(values: &VecDeque<Duration>) -> LatencySnapshot {
    if values.is_empty() {
        return LatencySnapshot::default();
    }
    let mut millis = values
        .iter()
        .map(|value| value.as_secs_f64() * 1000.0)
        .collect::<Vec<_>>();
    millis.sort_by(|left, right| left.total_cmp(right));
    let sum_ms = millis.iter().sum::<f64>();
    let avg_ms = sum_ms / millis.len() as f64;
    let buckets = latency_buckets(&millis);
    LatencySnapshot {
        p50_ms: percentile(&millis, 0.50),
        p95_ms: percentile(&millis, 0.95),
        avg_ms,
        count: millis.len() as u64,
        sum_ms,
        buckets,
    }
}

fn latency_buckets(sorted_millis: &[f64]) -> Vec<LatencyBucketSnapshot> {
    const BUCKETS_SECONDS: &[f64] = &[
        0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.000, 2.500, 5.000,
    ];

    BUCKETS_SECONDS
        .iter()
        .map(|le_seconds| LatencyBucketSnapshot {
            le_seconds: *le_seconds,
            count: sorted_millis
                .iter()
                .filter(|millis| **millis <= le_seconds * 1000.0)
                .count() as u64,
        })
        .collect()
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[index.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observer() -> Observer {
        Observer::new(100, 100, 100, 2, 2, 1)
    }

    #[test]
    fn records_shadow_match() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");
        let key = CacheKeyHash("key".to_string());
        let fp = ResponseFingerprint::new(200, "h".to_string(), Some("b".to_string()));

        for _ in 0..2 {
            observer.record(ObservationRecord {
                route_id: route.clone(),
                cache_key_hash: Some(key.clone()),
                decision: Decision::ObserveOnly,
                reasons: vec![DecisionReason::InsufficientShadowValidations],
                status: 200,
                latency: Duration::from_millis(1),
                origin: true,
                reused: false,
                protected: false,
                bypass: false,
                fingerprint: Some(fp.clone()),
                shadow_eligible: true,
                score: 90,
                mode: Mode::Shadow,
            });
        }

        let snapshot = observer.snapshot();
        assert_eq!(snapshot.overview.shadow_matches, 1);
    }

    #[test]
    fn records_shadow_mismatch_as_protected() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");
        let key = CacheKeyHash("key".to_string());

        for body_hash in ["a", "b"] {
            observer.record(ObservationRecord {
                route_id: route.clone(),
                cache_key_hash: Some(key.clone()),
                decision: Decision::ObserveOnly,
                reasons: vec![DecisionReason::InsufficientShadowValidations],
                status: 200,
                latency: Duration::from_millis(1),
                origin: true,
                reused: false,
                protected: false,
                bypass: false,
                fingerprint: Some(ResponseFingerprint::new(
                    200,
                    "h".to_string(),
                    Some(body_hash.to_string()),
                )),
                shadow_eligible: true,
                score: 90,
                mode: Mode::Shadow,
            });
        }

        assert_eq!(observer.route_state(&route), RouteState::Protected);
    }

    #[test]
    fn event_ring_buffer_is_bounded() {
        let observer = Observer::new(100, 100, 2, 2, 2, 1);

        for index in 0..4 {
            observer.push_event(
                EventType::StoreErrorFailOpen,
                None,
                None,
                vec![DecisionReason::StoreError],
                format!("event-{index}"),
            );
        }

        let snapshot = observer.snapshot();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.events[0].message, "event-2");
        assert_eq!(snapshot.events[1].message, "event-3");
    }

    #[test]
    fn route_promotes_to_auto_after_shadow_validation_threshold() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");
        let key = CacheKeyHash("key".to_string());
        let fp = ResponseFingerprint::new(200, "h".to_string(), Some("b".to_string()));

        for _ in 0..3 {
            observer.record(ObservationRecord {
                route_id: route.clone(),
                cache_key_hash: Some(key.clone()),
                decision: Decision::ObserveOnly,
                reasons: vec![DecisionReason::InsufficientShadowValidations],
                status: 200,
                latency: Duration::from_millis(1),
                origin: true,
                reused: false,
                protected: false,
                bypass: false,
                fingerprint: Some(fp.clone()),
                shadow_eligible: true,
                score: 90,
                mode: Mode::Auto,
            });
        }

        assert_eq!(observer.route_state(&route), RouteState::Auto);
        assert!(observer.is_auto_eligible(&route, &key));
    }
}
