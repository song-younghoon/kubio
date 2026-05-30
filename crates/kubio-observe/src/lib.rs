//! Process-local observation state for kubio.

use kubio_core::{
    CacheKeyHash, Decision, DecisionReason, HttpProtocol, LatencyBucketSnapshot, LatencySnapshot,
    Mode, ResponseFingerprint, RouteId, RouteState, StatusClass, StatusClassCounts,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, SystemTime};

const QUERY_VALUE_SAMPLE_LIMIT: usize = 32;
const QUERY_SUGGESTION_MIN_FINGERPRINTS: u64 = 2;

#[derive(Debug)]
pub struct Observer {
    inner: RwLock<ObserverInner>,
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
            inner: RwLock::new(ObserverInner::default()),
            max_routes,
            max_keys,
            max_events,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
        }
    }

    pub fn record(&self, record: ObservationRecord) -> ObservationOutcome {
        let mut inner = self.inner.write();
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

    pub fn record_revalidation(
        &self,
        route_id: RouteId,
        cache_key_hash: Option<CacheKeyHash>,
        outcome: RevalidationOutcome,
    ) {
        let mut inner = self.inner.write();
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id.clone()));
        route.revalidation_attempts += 1;
        let (event_type, reason, message) = match outcome {
            RevalidationOutcome::NotModified => {
                route.revalidation_not_modified += 1;
                (
                    EventType::ResponseRevalidatedNotModified,
                    DecisionReason::RevalidationNotModified,
                    "origin confirmed the stored response is still current",
                )
            }
            RevalidationOutcome::Modified => {
                route.revalidation_modified += 1;
                (
                    EventType::ResponseRevalidatedModified,
                    DecisionReason::RevalidationModified,
                    "origin returned new content during revalidation",
                )
            }
            RevalidationOutcome::Failed => {
                route.revalidation_failed += 1;
                (
                    EventType::ResponseRevalidationFailed,
                    DecisionReason::RevalidationFailed,
                    "origin revalidation failed",
                )
            }
            RevalidationOutcome::Skipped => (
                EventType::ResponseRevalidationFailed,
                DecisionReason::NoValidatorAvailable,
                "revalidation skipped because no validator was available",
            ),
        };
        self.push_event_locked(
            &mut inner,
            event_type,
            Some(route_id),
            cache_key_hash,
            vec![reason],
            message,
        );
    }

    pub fn record_stale(
        &self,
        route_id: RouteId,
        cache_key_hash: Option<CacheKeyHash>,
        served: bool,
        reason: DecisionReason,
    ) {
        let mut inner = self.inner.write();
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id.clone()));
        if served {
            route.stale_served += 1;
        } else {
            route.stale_denied += 1;
        }
        self.push_event_locked(
            &mut inner,
            if served {
                EventType::StaleResponseServed
            } else {
                EventType::StaleResponseDenied
            },
            Some(route_id),
            cache_key_hash,
            vec![reason],
            if served {
                "served a verified stale response during an origin error"
            } else {
                "stale response was not allowed"
            },
        );
    }

    pub fn record_query_params(&self, route_id: RouteId, params: Vec<QueryParamRecord>) {
        if params.is_empty() {
            return;
        }
        let mut inner = self.inner.write();
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id));
        for param in params {
            let stats = route
                .query_params
                .entry(param.name.clone())
                .or_insert_with(|| QueryParamStats::new(param.name.clone()));
            stats.record_seen(&param);
        }
    }

    pub fn record_query_fingerprint(
        &self,
        route_id: RouteId,
        params: &[QueryParamRecord],
        fingerprint: &ResponseFingerprint,
    ) {
        if params.is_empty() {
            return;
        }

        let mut inner = self.inner.write();
        let mut suggestions = Vec::new();
        {
            let route = inner
                .routes
                .entry(route_id.clone())
                .or_insert_with(|| RouteStats::new(route_id.clone()));
            let fingerprint_hash = response_fingerprint_hash(fingerprint);
            for param in params {
                let stats = route
                    .query_params
                    .entry(param.name.clone())
                    .or_insert_with(|| QueryParamStats::new(param.name.clone()));
                let had_suggestion = stats.suggestion().is_some();
                stats.record_fingerprint(param, &fingerprint_hash);
                if !had_suggestion
                    && stats.suggestion().is_some()
                    && !stats.suggestion_event_emitted
                {
                    stats.suggestion_event_emitted = true;
                    suggestions.push(stats.name.clone());
                    route.query_param_suggestions += 1;
                }
            }
        }

        for name in suggestions {
            self.push_event_locked(
                &mut inner,
                EventType::QueryParamSuggestionCreated,
                Some(route_id.clone()),
                None,
                vec![DecisionReason::QueryHintApplied],
                format!("query parameter `{name}` is a candidate for explicit ignore"),
            );
        }
    }

    pub fn record_route_hint(
        &self,
        route_id: RouteId,
        hint_name: String,
        applied: bool,
        reason: DecisionReason,
    ) {
        let mut inner = self.inner.write();
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id.clone()));
        route.route_hint = Some(hint_name);
        if applied {
            route.route_hint_applied += 1;
        } else {
            route.route_hint_rejected += 1;
        }
        self.push_event_locked(
            &mut inner,
            if applied {
                EventType::RouteHintApplied
            } else {
                EventType::RouteHintRejected
            },
            Some(route_id),
            None,
            vec![reason],
            if applied {
                "route hint applied"
            } else {
                "route hint rejected by safety policy"
            },
        );
    }

    pub fn record_query_hint(&self, route_id: RouteId, applied: bool, reason: DecisionReason) {
        let mut inner = self.inner.write();
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id.clone()));
        if applied {
            route.query_hint_applied += 1;
        } else {
            route.query_hint_rejected += 1;
        }
        self.push_event_locked(
            &mut inner,
            if applied {
                EventType::QueryHintApplied
            } else {
                EventType::QueryHintRejected
            },
            Some(route_id),
            None,
            vec![reason],
            if applied {
                "query hint applied to cache key construction"
            } else {
                "query hint was not used for this request"
            },
        );
    }

    pub fn record_downstream_protocol(&self, route_id: RouteId, protocol: HttpProtocol) {
        let mut inner = self.inner.write();
        inner.downstream_protocols.increment(protocol);
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id));
        route.downstream_protocols.increment(protocol);
    }

    pub fn record_upstream_protocol(&self, route_id: RouteId, protocol: HttpProtocol) {
        let mut inner = self.inner.write();
        inner.upstream_protocols.increment(protocol);
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id));
        route.upstream_protocols.increment(protocol);
    }

    pub fn record_backpressure_rejection(&self, route_id: RouteId, protocol: HttpProtocol) {
        let mut inner = self.inner.write();
        inner.backpressure_rejections += 1;
        inner.downstream_protocols.increment(protocol);
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id.clone()));
        route.downstream_protocols.increment(protocol);
        self.push_event_locked(
            &mut inner,
            EventType::BackpressureRejected,
            Some(route_id),
            None,
            vec![DecisionReason::LowEstimatedBenefit],
            "request rejected because the in-flight request limit was reached",
        );
    }

    pub fn record_in_flight(&self, current: usize, max: usize) {
        let mut inner = self.inner.write();
        inner.in_flight_requests = current as u64;
        inner.max_in_flight_requests = max as u64;
    }

    pub fn record_protocol_fallback(
        &self,
        route_id: RouteId,
        preferred: HttpProtocol,
        actual: HttpProtocol,
    ) {
        let mut inner = self.inner.write();
        inner.protocol_fallbacks += 1;
        self.push_event_locked(
            &mut inner,
            EventType::ProtocolFallback,
            Some(route_id),
            None,
            vec![DecisionReason::PolicyError],
            format!("origin protocol fallback from {preferred} to {actual}"),
        );
    }

    pub fn record_header_limit_rejection(&self, route_id: RouteId, protocol: HttpProtocol) {
        let mut inner = self.inner.write();
        inner.downstream_protocols.increment(protocol);
        let route = inner
            .routes
            .entry(route_id.clone())
            .or_insert_with(|| RouteStats::new(route_id.clone()));
        route.downstream_protocols.increment(protocol);
        self.push_event_locked(
            &mut inner,
            EventType::RequestHeaderLimitExceeded,
            Some(route_id),
            None,
            vec![DecisionReason::HeaderListTooLarge],
            "request rejected because HTTP/2 headers exceeded the configured limit",
        );
    }

    pub fn route_state(&self, route_id: &RouteId) -> RouteState {
        self.inner
            .read()
            .routes
            .get(route_id)
            .map(|route| route.state)
            .unwrap_or(RouteState::Watching)
    }

    pub fn is_auto_eligible(&self, route_id: &RouteId, key_hash: &CacheKeyHash) -> bool {
        let inner = self.inner.read();
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
        let inner = self.inner.read().clone();
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

        let mut overview = OverviewSnapshot::from_routes(&routes);
        overview.store_errors = inner.store_errors;
        overview.dropped_events = inner.dropped_events;
        overview.backpressure_rejections = inner.backpressure_rejections;
        overview.protocol_fallbacks = inner.protocol_fallbacks;
        overview.in_flight_requests = inner.in_flight_requests;
        overview.max_in_flight_requests = inner.max_in_flight_requests;
        overview.downstream_http1_requests = inner.downstream_protocols.http1;
        overview.downstream_http2_requests = inner.downstream_protocols.http2;
        overview.downstream_http3_requests = inner.downstream_protocols.http3;
        overview.upstream_http1_requests = inner.upstream_protocols.http1;
        overview.upstream_http2_requests = inner.upstream_protocols.http2;
        overview.upstream_http3_requests = inner.upstream_protocols.http3;
        ObserverSnapshot {
            overview,
            routes,
            events: inner.events.into_iter().collect(),
        }
    }

    pub fn route_by_hash(&self, route_hash: &str) -> Option<RouteSnapshot> {
        self.inner
            .read()
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
        if matches!(
            event_type,
            EventType::StoreErrorFailOpen
                | EventType::DiskStoreErrorFailOpen
                | EventType::DiskStoreCorruptEntrySkipped
        ) {
            inner.store_errors += 1;
        }
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
            inner.dropped_events += 1;
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
        let mut inner = self.inner.write();
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

#[derive(Debug, Clone, Default)]
struct ObserverInner {
    routes: HashMap<RouteId, RouteStats>,
    keys: HashMap<CacheKeyHash, KeyObservation>,
    events: VecDeque<Event>,
    store_errors: u64,
    dropped_events: u64,
    backpressure_rejections: u64,
    protocol_fallbacks: u64,
    in_flight_requests: u64,
    max_in_flight_requests: u64,
    downstream_protocols: ProtocolCounts,
    upstream_protocols: ProtocolCounts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProtocolCounts {
    pub http1: u64,
    pub http2: u64,
    pub http3: u64,
}

impl ProtocolCounts {
    fn increment(&mut self, protocol: HttpProtocol) {
        match protocol {
            HttpProtocol::Http1 => self.http1 += 1,
            HttpProtocol::Http2 => self.http2 += 1,
            HttpProtocol::Http3 => self.http3 += 1,
        }
    }
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
    revalidation_attempts: u64,
    revalidation_not_modified: u64,
    revalidation_modified: u64,
    revalidation_failed: u64,
    stale_served: u64,
    stale_denied: u64,
    route_hint: Option<String>,
    route_hint_applied: u64,
    route_hint_rejected: u64,
    query_hint_applied: u64,
    query_hint_rejected: u64,
    query_param_suggestions: u64,
    downstream_protocols: ProtocolCounts,
    upstream_protocols: ProtocolCounts,
    status_classes: StatusClassCounts,
    latencies: VecDeque<Duration>,
    score: i16,
    reasons: Vec<DecisionReason>,
    query_params: HashMap<String, QueryParamStats>,
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
            revalidation_attempts: 0,
            revalidation_not_modified: 0,
            revalidation_modified: 0,
            revalidation_failed: 0,
            stale_served: 0,
            stale_denied: 0,
            route_hint: None,
            route_hint_applied: 0,
            route_hint_rejected: 0,
            query_hint_applied: 0,
            query_hint_rejected: 0,
            query_param_suggestions: 0,
            downstream_protocols: ProtocolCounts::default(),
            upstream_protocols: ProtocolCounts::default(),
            status_classes: StatusClassCounts::default(),
            latencies: VecDeque::new(),
            score: 0,
            reasons: Vec::new(),
            query_params: HashMap::new(),
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
            revalidation_attempts: self.revalidation_attempts,
            revalidation_not_modified: self.revalidation_not_modified,
            revalidation_modified: self.revalidation_modified,
            revalidation_failed: self.revalidation_failed,
            stale_served: self.stale_served,
            stale_denied: self.stale_denied,
            route_hint_applied: self.route_hint_applied,
            route_hint_rejected: self.route_hint_rejected,
            query_hint_applied: self.query_hint_applied,
            query_hint_rejected: self.query_hint_rejected,
            query_param_suggestions: self.query_param_suggestions,
            downstream_protocols: self.downstream_protocols.clone(),
            upstream_protocols: self.upstream_protocols.clone(),
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
            route_hint: self.route_hint.clone(),
            query_params: self
                .query_params
                .values()
                .map(QueryParamStats::snapshot)
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
struct QueryParamStats {
    name: String,
    seen_count: u64,
    configured_action: String,
    fingerprint_sensitive: bool,
    sensitive: bool,
    value_hashes: HashSet<String>,
    value_hash_overflow: bool,
    fingerprints_by_value: HashMap<String, String>,
    fingerprint_hashes: HashSet<String>,
    fingerprint_observations: u64,
    suggestion_event_emitted: bool,
}

impl QueryParamStats {
    fn new(name: String) -> Self {
        Self {
            name,
            seen_count: 0,
            configured_action: "observe".to_string(),
            fingerprint_sensitive: false,
            sensitive: false,
            value_hashes: HashSet::new(),
            value_hash_overflow: false,
            fingerprints_by_value: HashMap::new(),
            fingerprint_hashes: HashSet::new(),
            fingerprint_observations: 0,
            suggestion_event_emitted: false,
        }
    }

    fn record_seen(&mut self, param: &QueryParamRecord) {
        self.seen_count += 1;
        self.configured_action.clone_from(&param.configured_action);
        self.sensitive |= param.sensitive;
        if self.sensitive {
            return;
        }
        if let Some(value_hash) = param.value_hash.as_ref() {
            if self.value_hashes.contains(value_hash) {
                return;
            }
            if self.value_hashes.len() < QUERY_VALUE_SAMPLE_LIMIT {
                self.value_hashes.insert(value_hash.clone());
            } else {
                self.value_hash_overflow = true;
            }
        }
    }

    fn record_fingerprint(&mut self, param: &QueryParamRecord, fingerprint_hash: &str) {
        self.configured_action.clone_from(&param.configured_action);
        self.sensitive |= param.sensitive;
        if self.sensitive {
            return;
        }
        self.fingerprint_observations += 1;
        if !self.fingerprint_hashes.contains(fingerprint_hash) {
            if !self.fingerprint_hashes.is_empty() {
                self.fingerprint_sensitive = true;
            }
            if self.fingerprint_hashes.len() < QUERY_VALUE_SAMPLE_LIMIT {
                self.fingerprint_hashes.insert(fingerprint_hash.to_string());
            }
        }
        let Some(value_hash) = param.value_hash.as_ref() else {
            return;
        };
        if let Some(previous) = self.fingerprints_by_value.get(value_hash) {
            if previous != fingerprint_hash {
                self.fingerprint_sensitive = true;
            }
        } else if self.fingerprints_by_value.len() < QUERY_VALUE_SAMPLE_LIMIT {
            self.fingerprints_by_value
                .insert(value_hash.clone(), fingerprint_hash.to_string());
        }
        if self.value_count() > 1 && self.fingerprint_hashes.len() > 1 {
            self.fingerprint_sensitive = true;
        }
    }

    fn value_count(&self) -> usize {
        self.value_hashes.len()
            + usize::from(self.value_hash_overflow && self.value_hashes.len() < usize::MAX)
    }

    fn cardinality(&self) -> &'static str {
        if self.sensitive || (self.value_hashes.is_empty() && !self.value_hash_overflow) {
            "unknown"
        } else if self.value_hash_overflow || self.value_hashes.len() > 16 {
            "high"
        } else if self.value_hashes.len() > 4 {
            "medium"
        } else if self.value_hashes.len() > 1 {
            "low"
        } else {
            "one"
        }
    }

    fn suggestion(&self) -> Option<String> {
        if self.configured_action != "observe"
            || self.sensitive
            || self.fingerprint_sensitive
            || self.fingerprint_observations < QUERY_SUGGESTION_MIN_FINGERPRINTS
            || self.fingerprint_hashes.len() > 1
        {
            return None;
        }
        let known_noise = self.name.starts_with("utm_")
            || matches!(self.name.as_str(), "gclid" | "fbclid" | "mc_cid" | "mc_eid");
        let fragmented = matches!(self.cardinality(), "medium" | "high");
        if known_noise || fragmented {
            Some("candidate_ignore".to_string())
        } else {
            None
        }
    }

    fn snapshot(&self) -> QueryParamSnapshot {
        QueryParamSnapshot {
            name: self.name.clone(),
            seen_count: self.seen_count,
            cardinality: self.cardinality().to_string(),
            fingerprint_sensitive: self.fingerprint_sensitive,
            configured_action: self.configured_action.clone(),
            suggestion: self.suggestion(),
        }
    }
}

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
    pub revalidation_attempts: u64,
    pub revalidation_not_modified: u64,
    pub revalidation_modified: u64,
    pub revalidation_failed: u64,
    pub stale_responses_served: u64,
    pub stale_responses_denied: u64,
    pub route_hints_applied: u64,
    pub route_hints_rejected: u64,
    pub query_hints_applied: u64,
    pub query_hints_rejected: u64,
    pub query_param_suggestions: u64,
    pub store_errors: u64,
    pub dropped_events: u64,
    pub backpressure_rejections: u64,
    pub protocol_fallbacks: u64,
    pub in_flight_requests: u64,
    pub max_in_flight_requests: u64,
    pub downstream_http1_requests: u64,
    pub downstream_http2_requests: u64,
    pub downstream_http3_requests: u64,
    pub upstream_http1_requests: u64,
    pub upstream_http2_requests: u64,
    pub upstream_http3_requests: u64,
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
            overview.revalidation_attempts += route.revalidation_attempts;
            overview.revalidation_not_modified += route.revalidation_not_modified;
            overview.revalidation_modified += route.revalidation_modified;
            overview.revalidation_failed += route.revalidation_failed;
            overview.stale_responses_served += route.stale_served;
            overview.stale_responses_denied += route.stale_denied;
            overview.route_hints_applied += route.route_hint_applied;
            overview.route_hints_rejected += route.route_hint_rejected;
            overview.query_hints_applied += route.query_hint_applied;
            overview.query_hints_rejected += route.query_hint_rejected;
            overview.query_param_suggestions += route.query_param_suggestions;
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
    pub revalidation_attempts: u64,
    pub revalidation_not_modified: u64,
    pub revalidation_modified: u64,
    pub revalidation_failed: u64,
    pub stale_served: u64,
    pub stale_denied: u64,
    pub route_hint_applied: u64,
    pub route_hint_rejected: u64,
    pub query_hint_applied: u64,
    pub query_hint_rejected: u64,
    pub query_param_suggestions: u64,
    pub downstream_protocols: ProtocolCounts,
    pub upstream_protocols: ProtocolCounts,
    pub status_classes: StatusClassCounts,
    pub latency: LatencySnapshot,
    pub repeat_rate: f64,
    pub estimated_savings: f64,
    pub actual_reuse_rate: f64,
    pub score: i16,
    pub reasons: Vec<DecisionReason>,
    pub explanation: Vec<String>,
    pub route_hint: Option<String>,
    pub query_params: Vec<QueryParamSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParamSnapshot {
    pub name: String,
    pub seen_count: u64,
    pub cardinality: String,
    pub fingerprint_sensitive: bool,
    pub configured_action: String,
    pub suggestion: Option<String>,
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
    ResponseRevalidatedNotModified,
    ResponseRevalidatedModified,
    ResponseRevalidationFailed,
    StaleResponseServed,
    StaleResponseDenied,
    RouteHintApplied,
    RouteHintRejected,
    QueryHintApplied,
    QueryHintRejected,
    QueryParamSuggestionCreated,
    BackpressureRejected,
    ProtocolFallback,
    RequestHeaderLimitExceeded,
    StoreSaturated,
    DiskStoreOpened,
    DiskStoreCorruptEntrySkipped,
    DiskStoreErrorFailOpen,
}

fn response_fingerprint_hash(fingerprint: &ResponseFingerprint) -> String {
    format!(
        "{}:{}:{}",
        fingerprint.status,
        fingerprint.header_hash,
        fingerprint.body_hash.as_deref().unwrap_or("")
    )
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
        assert_eq!(snapshot.overview.store_errors, 4);
        assert_eq!(snapshot.overview.dropped_events, 2);
    }

    #[test]
    fn protocol_and_backpressure_counts_are_bounded() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");

        observer.record_downstream_protocol(route.clone(), HttpProtocol::Http2);
        observer.record_upstream_protocol(route.clone(), HttpProtocol::Http1);
        observer.record_backpressure_rejection(route.clone(), HttpProtocol::Http2);
        observer.record_in_flight(1, 2);
        observer.record_protocol_fallback(route.clone(), HttpProtocol::Http2, HttpProtocol::Http1);
        observer.record_header_limit_rejection(route.clone(), HttpProtocol::Http2);

        let snapshot = observer.snapshot();
        assert_eq!(snapshot.overview.downstream_http2_requests, 3);
        assert_eq!(snapshot.overview.upstream_http1_requests, 1);
        assert_eq!(snapshot.overview.backpressure_rejections, 1);
        assert_eq!(snapshot.overview.in_flight_requests, 1);
        assert_eq!(snapshot.overview.max_in_flight_requests, 2);
        assert_eq!(snapshot.overview.protocol_fallbacks, 1);
        assert!(snapshot
            .events
            .iter()
            .any(|event| event.event_type == EventType::BackpressureRejected));
        assert!(snapshot
            .events
            .iter()
            .any(|event| event.event_type == EventType::ProtocolFallback));
        assert!(snapshot
            .events
            .iter()
            .any(|event| event.event_type == EventType::RequestHeaderLimitExceeded));
        let route = snapshot
            .routes
            .iter()
            .find(|route| route.route_id.as_label() == "GET /api/products")
            .unwrap();
        assert_eq!(route.downstream_protocols.http2, 3);
        assert_eq!(route.upstream_protocols.http1, 1);
    }

    #[test]
    fn query_stats_track_cardinality_and_suggestion_without_values() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");
        let params = ["a", "b", "c"]
            .into_iter()
            .map(|value| QueryParamRecord {
                name: "utm_source".to_string(),
                configured_action: "observe".to_string(),
                value_hash: Some(value.to_string()),
                sensitive: false,
            })
            .collect::<Vec<_>>();
        let fp = ResponseFingerprint::new(200, "h".to_string(), Some("stable".to_string()));

        for param in &params {
            observer.record_query_params(route.clone(), vec![param.clone()]);
            observer.record_query_fingerprint(route.clone(), std::slice::from_ref(param), &fp);
        }

        let snapshot = observer.snapshot();
        let route = snapshot
            .routes
            .iter()
            .find(|route| route.route_id.template == "/api/products")
            .unwrap();
        let param = route
            .query_params
            .iter()
            .find(|param| param.name == "utm_source")
            .unwrap();
        assert_eq!(param.cardinality, "low");
        assert!(!param.fingerprint_sensitive);
        assert_eq!(param.suggestion.as_deref(), Some("candidate_ignore"));
        assert_eq!(snapshot.overview.query_param_suggestions, 1);
    }

    #[test]
    fn query_stats_mark_fingerprint_sensitive_params() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");

        for (value_hash, body_hash) in [("a", "body-a"), ("b", "body-b")] {
            let param = QueryParamRecord {
                name: "variant".to_string(),
                configured_action: "observe".to_string(),
                value_hash: Some(value_hash.to_string()),
                sensitive: false,
            };
            observer.record_query_params(route.clone(), vec![param.clone()]);
            observer.record_query_fingerprint(
                route.clone(),
                &[param],
                &ResponseFingerprint::new(200, "h".to_string(), Some(body_hash.to_string())),
            );
        }

        let snapshot = observer.snapshot();
        let route = snapshot
            .routes
            .iter()
            .find(|route| route.route_id.template == "/api/products")
            .unwrap();
        let param = route
            .query_params
            .iter()
            .find(|param| param.name == "variant")
            .unwrap();
        assert!(param.fingerprint_sensitive);
        assert_eq!(param.suggestion, None);
    }

    #[test]
    fn query_stats_keep_value_and_fingerprint_samples_bounded() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");
        let fp = ResponseFingerprint::new(200, "h".to_string(), Some("stable".to_string()));

        for index in 0..(QUERY_VALUE_SAMPLE_LIMIT + 8) {
            let param = QueryParamRecord {
                name: "utm_source".to_string(),
                configured_action: "observe".to_string(),
                value_hash: Some(format!("value-{index}")),
                sensitive: false,
            };
            observer.record_query_params(route.clone(), vec![param.clone()]);
            observer.record_query_fingerprint(route.clone(), &[param], &fp);
        }

        let inner = observer.inner.read();
        let param = inner
            .routes
            .get(&route)
            .unwrap()
            .query_params
            .get("utm_source")
            .unwrap();
        assert_eq!(param.value_hashes.len(), QUERY_VALUE_SAMPLE_LIMIT);
        assert_eq!(param.fingerprints_by_value.len(), QUERY_VALUE_SAMPLE_LIMIT);
        assert_eq!(param.fingerprint_hashes.len(), 1);
        assert!(param.value_hash_overflow);
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
