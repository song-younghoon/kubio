use kubio_core::{
    query_pattern_matches, short_hash, AdaptiveReuseBlocker, AdaptiveReuseConfig, CacheKeyHash,
    Decision, DecisionReason, HttpProtocol, Mode, PathObservation, ResponseFingerprint, ReuseClass,
    RouteId, RouteQueryConfig, RouteState, StatusClass,
};
use parking_lot::RwLock;
use std::time::{Duration, SystemTime};

use crate::events::{Event, EventType};
use crate::protocol::{AltSvcOutcome, AltSvcReason, Http3ServerEvent, UpstreamHttp3Event};
use crate::query::{response_fingerprint_hash, QueryParamStats};
use crate::records::{
    KeyObservation, ObservationOutcome, ObservationRecord, QueryParamRecord, RevalidationOutcome,
};
use crate::snapshot::{state_sort_key, ObserverSnapshot, OverviewSnapshot, RouteSnapshot};
use crate::state::{ObserverInner, RouteStats};

#[derive(Debug)]
pub struct Observer {
    inner: RwLock<ObserverInner>,
    max_routes: usize,
    max_keys: usize,
    max_events: usize,
    min_route_samples: u64,
    min_key_repeats: u64,
    min_shadow_validations: u64,
    adaptive_reuse: AdaptiveReuseConfig,
}

#[derive(Debug, Clone)]
pub struct ReuseEligibility {
    pub eligible: bool,
    pub reuse_class: ReuseClass,
    pub blockers: Vec<AdaptiveReuseBlocker>,
}

impl ReuseEligibility {
    fn eligible(reuse_class: ReuseClass) -> Self {
        Self {
            eligible: true,
            reuse_class,
            blockers: Vec::new(),
        }
    }

    fn blocked(reuse_class: ReuseClass, blockers: Vec<AdaptiveReuseBlocker>) -> Self {
        Self {
            eligible: false,
            reuse_class,
            blockers,
        }
    }
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
        Self::with_adaptive_config(
            max_routes,
            max_keys,
            max_events,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
            AdaptiveReuseConfig::from_legacy_thresholds(
                min_route_samples,
                min_key_repeats,
                min_shadow_validations,
            ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_adaptive_config(
        max_routes: usize,
        max_keys: usize,
        max_events: usize,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
        adaptive_reuse: AdaptiveReuseConfig,
    ) -> Self {
        Self {
            inner: RwLock::new(ObserverInner::default()),
            max_routes,
            max_keys,
            max_events,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
            adaptive_reuse,
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
            if record.origin && record.shadow_eligible {
                route.store_safe_count += 1;
            }
            if record.origin {
                let now = SystemTime::now();
                if record.shadow_eligible {
                    route.precision_positive_samples += 1;
                    if route.first_evidence_at.is_none() {
                        route.first_evidence_at = Some(now);
                    }
                    route.last_evidence_at = Some(now);
                } else if record.protected {
                    route.precision_negative_samples += 1;
                    route.last_evidence_at = Some(now);
                }
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

            if let Some(route) = inner.routes.get_mut(&record.route_id) {
                route.distinct_key_hashes.insert(key_hash.clone());
            }

            if outcome.shadow_match || outcome.shadow_mismatch {
                if let Some(route) = inner.routes.get_mut(&record.route_id) {
                    if outcome.shadow_match {
                        route.shadow_matches += 1;
                    }
                    if outcome.shadow_mismatch {
                        route.shadow_mismatches += 1;
                        route.precision_negative_samples += 1;
                        let now = SystemTime::now();
                        route.last_evidence_at = Some(now);
                        let cooldown = self
                            .adaptive_reuse
                            .precision
                            .confidence
                            .cooldown_secs
                            .min(self.adaptive_reuse.precision.confidence.max_cooldown_secs);
                        route.cooldown_until = Some(now + Duration::from_secs(cooldown));
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

    pub fn record_path_observation(&self, route_id: RouteId, observation: PathObservation) {
        let mut inner = self.inner.write();
        let mut slug_event_route = None;
        {
            let route = inner
                .routes
                .entry(route_id)
                .or_insert_with_key(|route_id| RouteStats::new(route_id.clone()));
            if observation.sensitive_path_score > 0 {
                route.path_sensitive_samples += 1;
            }
            if observation.id_like_segment_count > 0 {
                route.id_like_path_samples += 1;
            }
            if observation.slug_like_segment_count > 0 {
                route.slug_like_path_samples += 1;
                slug_event_route = Some(route.route_id.clone());
            }
            for hash in observation.dynamic_segment_hashes {
                if route.dynamic_segment_hashes.len() < 1024 {
                    route.dynamic_segment_hashes.insert(hash);
                }
            }
            for hash in observation.slug_segment_hashes {
                if route.slug_segment_hashes.len() < 1024 {
                    route.slug_segment_hashes.insert(hash);
                }
            }
        }
        if let Some(route_id) = slug_event_route {
            self.push_event_locked(
                &mut inner,
                EventType::SlugRouteCandidateDetected,
                Some(route_id),
                None,
                vec![DecisionReason::PublicObjectValidated],
                "slug-like path evidence observed for a public object route",
            );
        }
    }

    pub fn record_origin_public_response(
        &self,
        route_id: RouteId,
        cache_key_hash: Option<CacheKeyHash>,
        mode: Mode,
    ) {
        let mut inner = self.inner.write();
        let mut event = None;
        {
            let route = inner
                .routes
                .entry(route_id.clone())
                .or_insert_with(|| RouteStats::new(route_id.clone()));
            route.origin_public_responses += 1;
            if self.adaptive_reuse.enabled
                && self.adaptive_reuse.origin_public_fast_path.enabled
                && route.state != RouteState::Protected
                && route.shadow_mismatches == 0
                && mode == Mode::Auto
                && route.state != RouteState::Auto
            {
                route.state = RouteState::Auto;
                event = Some((
                    EventType::RoutePromotedToAuto,
                    vec![DecisionReason::OriginPublicCacheControl],
                    "route promoted because origin responses are explicitly public",
                ));
            }
        }
        if let Some((event_type, reasons, message)) = event {
            self.push_event_locked(
                &mut inner,
                event_type,
                Some(route_id),
                cache_key_hash,
                reasons,
                message,
            );
        }
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
        let mut verified = Vec::new();
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
                let was_verified = stats
                    .verified_ignore_candidate(&self.adaptive_reuse.precision.query_equivalence);
                stats.record_fingerprint(param, &fingerprint_hash);
                if !had_suggestion
                    && stats.suggestion().is_some()
                    && !stats.suggestion_event_emitted
                {
                    stats.suggestion_event_emitted = true;
                    suggestions.push(stats.name.clone());
                    route.query_param_suggestions += 1;
                }
                if !was_verified
                    && stats
                        .verified_ignore_candidate(&self.adaptive_reuse.precision.query_equivalence)
                {
                    verified.push(stats.name.clone());
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
        for name in verified {
            self.push_event_locked(
                &mut inner,
                EventType::QueryEquivalenceCandidateVerified,
                Some(route_id.clone()),
                None,
                vec![DecisionReason::QueryHintApplied],
                format!("query parameter `{name}` has matching fingerprint evidence for verified ignore"),
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

    pub fn record_variant_values(&self, route_id: RouteId, variants: Vec<(String, String)>) {
        if variants.is_empty() {
            return;
        }
        let mut inner = self.inner.write();
        let mut unbounded = false;
        {
            let route = inner
                .routes
                .entry(route_id.clone())
                .or_insert_with(|| RouteStats::new(route_id.clone()));
            for (name, value_hash) in variants {
                let values = route.variant_value_hashes.entry(name).or_default();
                if values.len() < 64 {
                    values.insert(value_hash);
                }
                if values.len() as u64
                    > self
                        .adaptive_reuse
                        .precision
                        .variants
                        .max_values_per_dimension
                {
                    unbounded = true;
                }
            }
        }
        if unbounded {
            self.push_event_locked(
                &mut inner,
                EventType::VariantUnboundedDetected,
                Some(route_id),
                None,
                vec![DecisionReason::VaryUnsupported],
                "configured variant dimension exceeded the bounded value limit",
            );
        }
    }

    pub fn verified_query_ignores(
        &self,
        route_id: &RouteId,
        query_config: Option<&RouteQueryConfig>,
    ) -> Vec<String> {
        if !self.adaptive_reuse.enabled
            || !self.adaptive_reuse.precision.enabled
            || !self.adaptive_reuse.precision.query_equivalence.enabled
        {
            return Vec::new();
        }
        let route_enabled = query_config
            .map(|config| config.verified_ignore.enabled)
            .unwrap_or(false);
        let allow_patterns = query_config
            .map(|config| config.verified_ignore.allow.as_slice())
            .unwrap_or(&[]);
        let auto_compact = self.adaptive_reuse.precision.query_equivalence.auto_compact;
        if !route_enabled && !auto_compact {
            return Vec::new();
        }

        let mut compacted = Vec::new();
        let mut inner = self.inner.write();
        let mut events = Vec::new();
        if let Some(route) = inner.routes.get_mut(route_id) {
            for stats in route.query_params.values() {
                if !stats
                    .verified_ignore_candidate(&self.adaptive_reuse.precision.query_equivalence)
                {
                    continue;
                }
                let allowed_by_route = route_enabled
                    && allow_patterns
                        .iter()
                        .any(|pattern| query_pattern_matches(pattern, &stats.name));
                let allowed_by_auto = auto_compact && known_tracking_query_name(&stats.name);
                if allowed_by_route || allowed_by_auto {
                    compacted.push(stats.name.clone());
                    if route.query_compacted_groups.insert(stats.name.clone()) {
                        events.push(stats.name.clone());
                    }
                }
            }
        }
        for name in events {
            self.push_event_locked(
                &mut inner,
                EventType::QueryEquivalenceCompactionApplied,
                Some(route_id.clone()),
                None,
                vec![DecisionReason::QueryHintApplied],
                format!("query parameter `{name}` is ignored after verified equivalence proof"),
            );
        }
        compacted.sort();
        compacted
    }

    pub fn should_canary_validate(&self, route_id: &RouteId, key_hash: &CacheKeyHash) -> bool {
        if !self.adaptive_reuse.enabled
            || !self.adaptive_reuse.precision.enabled
            || !self.adaptive_reuse.precision.canary.enabled
        {
            return false;
        }
        let mut inner = self.inner.write();
        let Some(route) = inner.routes.get_mut(route_id) else {
            return false;
        };
        let tier = route.confidence_tier(&self.adaptive_reuse);
        let rate = match tier {
            kubio_core::ConfidenceTier::Probation => {
                self.adaptive_reuse.precision.canary.probation_rate
            }
            kubio_core::ConfidenceTier::Validated => {
                self.adaptive_reuse.precision.canary.validated_rate
            }
            kubio_core::ConfidenceTier::Strong => self.adaptive_reuse.precision.canary.strong_rate,
            _ => return false,
        };
        if rate <= 0.0 {
            return false;
        }
        let now = SystemTime::now();
        if route
            .last_canary_at
            .and_then(|last| now.duration_since(last).ok())
            .map(|elapsed| {
                elapsed.as_secs() < self.adaptive_reuse.precision.canary.min_interval_secs
            })
            .unwrap_or(false)
        {
            return false;
        }
        let sample = deterministic_sample(route_id, key_hash);
        if sample < rate {
            route.last_canary_at = Some(now);
            true
        } else {
            false
        }
    }

    pub fn record_canary_validation(
        &self,
        route_id: RouteId,
        cache_key_hash: CacheKeyHash,
        matched: bool,
    ) {
        let mut inner = self.inner.write();
        let mut cooldown_event = false;
        {
            let route = inner
                .routes
                .entry(route_id.clone())
                .or_insert_with(|| RouteStats::new(route_id.clone()));
            let now = SystemTime::now();
            if matched {
                route.canary_matches += 1;
                route.precision_positive_samples += 1;
                route.last_evidence_at = Some(now);
                if route.first_evidence_at.is_none() {
                    route.first_evidence_at = Some(now);
                }
            } else {
                route.canary_mismatches += 1;
                route.precision_negative_samples += 1;
                route.shadow_mismatches += 1;
                route.state = RouteState::Protected;
                route.reasons = vec![DecisionReason::ShadowMismatch];
                let base = self.adaptive_reuse.precision.confidence.cooldown_secs;
                let max = self.adaptive_reuse.precision.confidence.max_cooldown_secs;
                let exponent = route.cooldown_count.min(8) as i32;
                let backed_off = (base as f64
                    * self
                        .adaptive_reuse
                        .precision
                        .confidence
                        .cooldown_backoff
                        .powi(exponent)) as u64;
                let cooldown = backed_off.min(max).max(base);
                route.cooldown_until = Some(now + Duration::from_secs(cooldown));
                route.cooldown_count += 1;
                route.last_evidence_at = Some(now);
                cooldown_event = true;
            }
        }
        self.push_event_locked(
            &mut inner,
            if matched {
                EventType::PrecisionCanaryMatch
            } else {
                EventType::PrecisionCanaryMismatch
            },
            Some(route_id.clone()),
            Some(cache_key_hash),
            if matched {
                vec![DecisionReason::ReusableAndFresh]
            } else {
                vec![DecisionReason::ShadowMismatch]
            },
            if matched {
                "precision canary matched the stored response fingerprint"
            } else {
                "precision canary found a fingerprint mismatch"
            },
        );
        if cooldown_event {
            self.push_event_locked(
                &mut inner,
                EventType::PrecisionCooldownStarted,
                Some(route_id),
                None,
                vec![DecisionReason::ShadowMismatch],
                "route entered precision cooldown after negative evidence",
            );
        }
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

    pub fn record_alt_svc(&self, route_id: RouteId, outcome: AltSvcOutcome, reason: AltSvcReason) {
        let mut inner = self.inner.write();
        inner.alt_svc.increment(outcome, reason);
        self.push_event_locked(
            &mut inner,
            if outcome == AltSvcOutcome::Advertised {
                EventType::AltSvcAdvertised
            } else {
                EventType::AltSvcSkipped
            },
            Some(route_id),
            None,
            vec![DecisionReason::PolicyError],
            if outcome == AltSvcOutcome::Advertised {
                "Alt-Svc advertised for a configured HTTP/3 authority"
            } else {
                reason.message()
            },
        );
    }

    pub fn record_http3_server_event(&self, event: Http3ServerEvent) {
        let mut inner = self.inner.write();
        inner.http3_server.increment(event);
        if matches!(
            event,
            Http3ServerEvent::HandshakeFailed
                | Http3ServerEvent::ResponseWriteHeadersFailed
                | Http3ServerEvent::ResponseWriteBodyFailed
                | Http3ServerEvent::ResponseFinishFailed
        ) {
            self.push_event_locked(
                &mut inner,
                EventType::Http3RuntimeError,
                None,
                None,
                vec![DecisionReason::PolicyError],
                event.message(),
            );
        }
    }

    pub fn record_upstream_http3_event(&self, route_id: RouteId, event: UpstreamHttp3Event) {
        let mut inner = self.inner.write();
        inner.upstream_http3.increment(event);
        if matches!(
            event,
            UpstreamHttp3Event::Fallback
                | UpstreamHttp3Event::Failure
                | UpstreamHttp3Event::RequiredFailure
                | UpstreamHttp3Event::SkippedNotHttps
                | UpstreamHttp3Event::SkippedNonReplayable
        ) {
            self.push_event_locked(
                &mut inner,
                if event == UpstreamHttp3Event::Fallback {
                    EventType::UpstreamHttp3Fallback
                } else {
                    EventType::UpstreamHttp3Failed
                },
                Some(route_id),
                None,
                vec![DecisionReason::PolicyError],
                event.message(),
            );
        }
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
        self.reuse_eligibility(route_id, key_hash, true, false)
            .eligible
    }

    pub fn reuse_eligibility(
        &self,
        route_id: &RouteId,
        key_hash: &CacheKeyHash,
        request_reuse_safe: bool,
        route_hint_public_object: bool,
    ) -> ReuseEligibility {
        self.adaptive_eligibility(
            route_id,
            key_hash,
            request_reuse_safe,
            route_hint_public_object,
            false,
        )
    }

    pub fn store_eligibility(
        &self,
        route_id: &RouteId,
        key_hash: &CacheKeyHash,
        request_reuse_safe: bool,
        route_hint_public_object: bool,
        origin_public_response: bool,
    ) -> ReuseEligibility {
        self.adaptive_eligibility(
            route_id,
            key_hash,
            request_reuse_safe,
            route_hint_public_object,
            origin_public_response,
        )
    }

    pub fn snapshot(&self) -> ObserverSnapshot {
        let inner = self.inner.read().clone();
        let mut routes = inner
            .routes
            .values()
            .map(|route| route.snapshot(&self.adaptive_reuse))
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
        overview.alt_svc = inner.alt_svc.clone();
        overview.http3_server = inner.http3_server.clone();
        overview.upstream_http3 = inner.upstream_http3.clone();
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
            .map(|route| route.snapshot(&self.adaptive_reuse))
    }

    fn adaptive_eligibility(
        &self,
        route_id: &RouteId,
        key_hash: &CacheKeyHash,
        request_reuse_safe: bool,
        route_hint_public_object: bool,
        origin_public_response: bool,
    ) -> ReuseEligibility {
        let inner = self.inner.read();
        if !self.adaptive_reuse.enabled {
            return self.legacy_eligibility_locked(&inner, route_id, key_hash);
        }
        if !request_reuse_safe {
            return ReuseEligibility::blocked(
                ReuseClass::HardProtected,
                vec![AdaptiveReuseBlocker::UnsafeRequest],
            );
        }

        let Some(route) = inner.routes.get(route_id) else {
            return ReuseEligibility::blocked(
                ReuseClass::Watching,
                vec![AdaptiveReuseBlocker::InsufficientRouteSamples],
            );
        };

        if route.cooldown_remaining_seconds().is_some() {
            return ReuseEligibility::blocked(
                ReuseClass::HardProtected,
                vec![AdaptiveReuseBlocker::CooldownActive],
            );
        }
        if route.state == RouteState::Protected {
            return ReuseEligibility::blocked(
                ReuseClass::HardProtected,
                vec![AdaptiveReuseBlocker::ProtectedRoute],
            );
        }
        if route.stale_evidence(&self.adaptive_reuse) {
            return ReuseEligibility::blocked(
                route.reuse_class(&self.adaptive_reuse),
                vec![AdaptiveReuseBlocker::StaleEvidence],
            );
        }
        if route.shadow_mismatches > self.adaptive_reuse.public_object.max_shadow_mismatches {
            return ReuseEligibility::blocked(
                ReuseClass::HardProtected,
                vec![AdaptiveReuseBlocker::ShadowMismatch],
            );
        }

        if self.key_validated_locked(&inner, key_hash) {
            return ReuseEligibility::eligible(ReuseClass::KeyValidated);
        }

        if self.adaptive_reuse.origin_public_fast_path.enabled
            && (origin_public_response || route.origin_public_responses > 0)
        {
            return ReuseEligibility::eligible(ReuseClass::OriginPublic);
        }

        if route_hint_public_object {
            return ReuseEligibility::eligible(ReuseClass::PublicObject);
        }

        if route.public_object_ready(&self.adaptive_reuse) {
            return ReuseEligibility::eligible(ReuseClass::PublicObject);
        }

        let key_blockers = self.key_blockers_locked(&inner, key_hash);
        let mut blockers = if key_blockers.is_empty() {
            route.eligibility_blockers(&self.adaptive_reuse)
        } else {
            key_blockers
        };
        if blockers.is_empty() {
            blockers.push(AdaptiveReuseBlocker::InsufficientShadowMatches);
        }
        ReuseEligibility::blocked(route.reuse_class(&self.adaptive_reuse), blockers)
    }

    fn legacy_eligibility_locked(
        &self,
        inner: &ObserverInner,
        route_id: &RouteId,
        key_hash: &CacheKeyHash,
    ) -> ReuseEligibility {
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
                    && (key.recent_shadow_matches as u64) >= self.min_shadow_validations
                    && key.recent_shadow_mismatches == 0
            })
            .unwrap_or(false);
        if route_ok && key_ok {
            ReuseEligibility::eligible(ReuseClass::KeyValidated)
        } else {
            ReuseEligibility::blocked(ReuseClass::Watching, vec![AdaptiveReuseBlocker::Disabled])
        }
    }

    fn key_validated_locked(&self, inner: &ObserverInner, key_hash: &CacheKeyHash) -> bool {
        let key_validation = &self.adaptive_reuse.key_validation;
        inner
            .keys
            .get(key_hash)
            .map(|key| {
                key.seen_count >= key_validation.min_observations
                    && (key.recent_shadow_matches as u64) >= key_validation.min_shadow_matches
                    && (key.recent_shadow_mismatches as u64) <= key_validation.max_shadow_mismatches
            })
            .unwrap_or(false)
    }

    fn key_blockers_locked(
        &self,
        inner: &ObserverInner,
        key_hash: &CacheKeyHash,
    ) -> Vec<AdaptiveReuseBlocker> {
        let key_validation = &self.adaptive_reuse.key_validation;
        let Some(key) = inner.keys.get(key_hash) else {
            return vec![AdaptiveReuseBlocker::InsufficientKeyObservations];
        };
        let mut blockers = Vec::new();
        if (key.recent_shadow_mismatches as u64) > key_validation.max_shadow_mismatches {
            blockers.push(AdaptiveReuseBlocker::ShadowMismatch);
        }
        if key.seen_count < key_validation.min_observations {
            blockers.push(AdaptiveReuseBlocker::InsufficientKeyObservations);
        }
        if (key.recent_shadow_matches as u64) < key_validation.min_shadow_matches {
            blockers.push(AdaptiveReuseBlocker::InsufficientShadowMatches);
        }
        blockers
    }

    fn update_route_state_locked(
        &self,
        inner: &mut ObserverInner,
        record: &ObservationRecord,
        outcome: &mut ObservationOutcome,
    ) {
        let mut event = None;
        let mut skip_legacy_promotion = false;
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

            if self.adaptive_reuse.enabled && route.public_object_ready(&self.adaptive_reuse) {
                if record.mode == Mode::Auto {
                    if route.state != RouteState::Auto {
                        route.state = RouteState::Auto;
                        outcome.promoted_to_auto = true;
                        event = Some((
                            EventType::RoutePromotedToAuto,
                            vec![DecisionReason::PublicObjectValidated],
                            "route promoted because path cardinality and stable responses indicate public objects",
                        ));
                    }
                } else if route.state != RouteState::ShadowValidated {
                    route.state = RouteState::ShadowValidated;
                    event = Some((
                        EventType::RoutePromotedToShadow,
                        vec![DecisionReason::PublicObjectValidated],
                        "route has enough evidence to be treated as public objects after auto mode is enabled",
                    ));
                }
                skip_legacy_promotion = true;
            }

            if !skip_legacy_promotion {
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

fn known_tracking_query_name(name: &str) -> bool {
    name.starts_with("utm_") || matches!(name, "gclid" | "fbclid" | "mc_cid" | "mc_eid")
}

fn deterministic_sample(route_id: &RouteId, key_hash: &CacheKeyHash) -> f64 {
    let material = format!("{}:{}", route_id.as_label(), key_hash);
    let hash = short_hash(&material);
    let prefix = hash.get(..8).unwrap_or(hash.as_str());
    let value = u64::from_str_radix(prefix, 16).unwrap_or_else(|_| {
        prefix.as_bytes().iter().fold(0_u64, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(*byte as u64)
        })
    });
    (value % 10_000) as f64 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::QUERY_VALUE_SAMPLE_LIMIT;

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
        assert_eq!(
            param.equivalence_class,
            kubio_core::QueryEquivalenceClass::VerifiedIgnoreCandidate
        );
        assert_eq!(snapshot.overview.query_param_suggestions, 1);
    }

    #[test]
    fn verified_query_ignores_require_operator_enablement() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");
        let fp = ResponseFingerprint::new(200, "h".to_string(), Some("stable".to_string()));

        for value_hash in ["a", "b", "c"] {
            let param = QueryParamRecord {
                name: "utm_source".to_string(),
                configured_action: "observe".to_string(),
                value_hash: Some(value_hash.to_string()),
                sensitive: false,
            };
            observer.record_query_params(route.clone(), vec![param.clone()]);
            observer.record_query_fingerprint(route.clone(), &[param], &fp);
        }

        assert!(observer.verified_query_ignores(&route, None).is_empty());
        let query = RouteQueryConfig {
            include: Vec::new(),
            ignore: Vec::new(),
            verified_ignore: kubio_core::RouteVerifiedIgnoreConfig {
                enabled: true,
                allow: vec!["utm_*".to_string()],
            },
        };
        assert_eq!(
            observer.verified_query_ignores(&route, Some(&query)),
            vec!["utm_source".to_string()]
        );
        let snapshot = observer.snapshot();
        let route = snapshot
            .routes
            .iter()
            .find(|route| route.route_id.template == "/api/products")
            .unwrap();
        assert_eq!(route.query_compacted_groups, 1);
    }

    #[test]
    fn canary_mismatch_enters_cooldown() {
        let observer = observer();
        let route = RouteId::new("GET", "/api/products");
        let key = CacheKeyHash("key".to_string());

        observer.record_canary_validation(route.clone(), key.clone(), false);

        let snapshot = observer.snapshot();
        let route_snapshot = snapshot
            .routes
            .iter()
            .find(|route_snapshot| route_snapshot.route_id == route)
            .unwrap();
        assert_eq!(
            route_snapshot.confidence_tier,
            kubio_core::ConfidenceTier::Cooldown
        );
        assert!(route_snapshot.cooldown_remaining_seconds.is_some());
        assert!(
            !observer
                .reuse_eligibility(&route, &key, true, false)
                .eligible
        );
    }

    #[test]
    fn stale_precision_evidence_blocks_adaptive_reuse() {
        let mut config = AdaptiveReuseConfig::default();
        config.precision.confidence.fresh_window_secs = 1;
        config.public_object.min_route_samples = 1;
        config.public_object.min_distinct_keys = 1;
        config.public_object.min_shadow_matches = 0;
        let observer = Observer::with_adaptive_config(100, 100, 100, 1, 1, 0, config);
        let route = RouteId::new("GET", "/notice/{id}");
        let key = CacheKeyHash("key".to_string());
        {
            let mut inner = observer.inner.write();
            let mut stats = RouteStats::new(route.clone());
            stats.request_count = 1;
            stats.origin_count = 1;
            stats.store_safe_count = 1;
            stats.distinct_key_hashes.insert(key.clone());
            stats.dynamic_segment_hashes.insert("id".to_string());
            stats.last_evidence_at = Some(SystemTime::now() - Duration::from_secs(5));
            inner.routes.insert(route.clone(), stats);
        }

        let eligibility = observer.reuse_eligibility(&route, &key, true, false);

        assert!(!eligibility.eligible);
        assert!(eligibility
            .blockers
            .contains(&AdaptiveReuseBlocker::StaleEvidence));
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
