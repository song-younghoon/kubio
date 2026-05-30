use crate::decision::observe_reasons;
use crate::headers::{bounded_header_value, parse_delta_seconds};
use crate::{
    CacheControlClass, ContentTypeClass, PolicyDecision, RequestSignals, ResponseSignals, VaryClass,
};
use http::{header, HeaderMap, Method, StatusCode};
use kubio_core::{
    sensitive_path_score, Decision, DecisionReason, EffectiveConfig, FreshnessProfile, Mode,
    PolicyConfig, RouteHintConfig, RouteState, StoredCacheControl, Validators,
};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    config: PolicyConfig,
    freshness: FreshnessProfile,
}

impl PolicyEngine {
    pub fn new(config: &EffectiveConfig) -> Self {
        Self {
            config: config.policy.clone(),
            freshness: config.freshness,
        }
    }

    pub fn config(&self) -> &PolicyConfig {
        &self.config
    }

    pub fn freshness_ttl(&self) -> Duration {
        self.freshness.ttl()
    }

    pub fn freshness_ttl_for_route(&self, hint: Option<&RouteHintConfig>) -> Duration {
        hint.and_then(|hint| hint.freshness.ttl)
            .unwrap_or_else(|| self.freshness_ttl())
    }

    pub fn request_signals(
        &self,
        method: &Method,
        path: &str,
        headers: &HeaderMap,
        body_len: usize,
    ) -> RequestSignals {
        RequestSignals {
            method_cacheable: method == Method::GET || method == Method::HEAD,
            has_authorization: headers.contains_key(header::AUTHORIZATION),
            has_cookie: headers.contains_key(header::COOKIE),
            has_range: headers.contains_key(header::RANGE),
            has_body_on_get_or_head: (method == Method::GET || method == Method::HEAD)
                && body_len > 0,
            query_param_count: 0,
            sensitive_path_score: sensitive_path_score(path),
        }
    }

    pub fn response_signals(&self, status: StatusCode, headers: &HeaderMap) -> ResponseSignals {
        ResponseSignals {
            status_cacheable: status == StatusCode::OK,
            has_set_cookie: headers.contains_key(header::SET_COOKIE),
            cache_control: CacheControlClass::from_headers(headers),
            vary: VaryClass::from_headers(headers),
            content_length: headers
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok()),
            content_type_class: ContentTypeClass::from_headers(headers),
        }
    }

    pub fn decide_request(
        &self,
        mode: Mode,
        route_state: RouteState,
        signals: &RequestSignals,
        panic_switch_active: bool,
    ) -> PolicyDecision {
        let mut reasons = Vec::new();

        if panic_switch_active {
            reasons.push(DecisionReason::PanicSwitchActive);
            return PolicyDecision::new(Decision::Bypass, reasons, route_state, 0);
        }

        if !signals.method_cacheable {
            reasons.push(DecisionReason::MethodNotCacheable);
        }
        if self.config.protect_authorization && signals.has_authorization {
            reasons.push(DecisionReason::HasAuthorization);
        }
        if self.config.protect_cookies && signals.has_cookie {
            reasons.push(DecisionReason::HasCookie);
        }
        if signals.has_range {
            reasons.push(DecisionReason::RangeRequest);
        }
        if signals.has_body_on_get_or_head {
            reasons.push(DecisionReason::RequestBodyOnGet);
        }
        if signals.sensitive_path_score > 0 {
            reasons.push(DecisionReason::SensitivePath);
        }

        if !reasons.is_empty() {
            return PolicyDecision::new(Decision::Protect, reasons, RouteState::Protected, -100);
        }

        match mode {
            Mode::Watch | Mode::Shadow => PolicyDecision::new(
                Decision::ObserveOnly,
                vec![DecisionReason::InsufficientShadowValidations],
                route_state,
                self.score(signals, None, false, false),
            ),
            Mode::Auto => {
                if route_state == RouteState::Auto {
                    PolicyDecision::new(
                        Decision::ObserveOnly,
                        vec![DecisionReason::ReusableAndFresh],
                        route_state,
                        self.score(signals, None, true, true),
                    )
                } else {
                    PolicyDecision::new(
                        Decision::ObserveOnly,
                        vec![DecisionReason::InsufficientShadowValidations],
                        route_state,
                        self.score(signals, None, false, false),
                    )
                }
            }
        }
    }

    pub fn decide_response(
        &self,
        mode: Mode,
        route_state: RouteState,
        request: &RequestSignals,
        response: &ResponseSignals,
        body_len: usize,
        fingerprint_available: bool,
    ) -> PolicyDecision {
        let mut reasons = self.response_hard_deny_reasons(response);
        if !response.status_cacheable {
            reasons.push(DecisionReason::StatusNotCacheable);
        }
        if response.cache_control == CacheControlClass::NoCache {
            reasons.push(DecisionReason::NoCacheRequiresRevalidation);
        }
        if body_len as u64 > self.config.max_object_size {
            reasons.push(DecisionReason::ObjectTooLarge);
        }
        if !fingerprint_available {
            reasons.push(DecisionReason::FingerprintUnavailable);
        }

        let hard_denied = reasons.iter().any(|reason| {
            matches!(
                reason,
                DecisionReason::HasSetCookie
                    | DecisionReason::CacheControlNoStore
                    | DecisionReason::CacheControlPrivate
                    | DecisionReason::VaryUnsupported
                    | DecisionReason::VaryWildcard
                    | DecisionReason::StatusNotCacheable
            )
        });

        let score = self.score(
            request,
            Some(response),
            route_state == RouteState::Auto,
            true,
        );

        if hard_denied {
            return PolicyDecision::new(Decision::Protect, reasons, RouteState::Protected, score);
        }

        if body_len as u64 > self.config.max_object_size || !fingerprint_available {
            return PolicyDecision::new(
                Decision::ObserveOnly,
                observe_reasons(reasons),
                route_state,
                score,
            );
        }

        match mode {
            Mode::Watch | Mode::Shadow => PolicyDecision::new(
                Decision::ObserveOnly,
                observe_reasons(reasons),
                route_state,
                score,
            ),
            Mode::Auto if route_state == RouteState::Auto => PolicyDecision::new(
                Decision::StoreOnly,
                vec![DecisionReason::ReusableAndFresh],
                route_state,
                score,
            ),
            Mode::Auto => PolicyDecision::new(
                Decision::ObserveOnly,
                observe_reasons(reasons),
                route_state,
                score,
            ),
        }
    }

    pub fn response_hard_deny_reasons(&self, response: &ResponseSignals) -> Vec<DecisionReason> {
        let mut reasons = Vec::new();
        if self.config.protect_set_cookie && response.has_set_cookie {
            reasons.push(DecisionReason::HasSetCookie);
        }
        match response.cache_control {
            CacheControlClass::NoStore => reasons.push(DecisionReason::CacheControlNoStore),
            CacheControlClass::Private => reasons.push(DecisionReason::CacheControlPrivate),
            CacheControlClass::NoCache => {}
            CacheControlClass::Absent | CacheControlClass::Public | CacheControlClass::Other => {}
        }
        match &response.vary {
            VaryClass::Wildcard => reasons.push(DecisionReason::VaryWildcard),
            VaryClass::Unsupported(_) => reasons.push(DecisionReason::VaryUnsupported),
            VaryClass::Absent | VaryClass::Supported(_) => {}
        }
        reasons
    }

    pub fn request_is_reuse_safe(&self, signals: &RequestSignals) -> bool {
        signals.method_cacheable
            && !signals.has_authorization
            && !signals.has_cookie
            && !signals.has_range
            && !signals.has_body_on_get_or_head
            && signals.sensitive_path_score == 0
    }

    pub fn response_is_store_safe(&self, response: &ResponseSignals) -> bool {
        response.status_cacheable && self.response_hard_deny_reasons(response).is_empty()
    }

    pub fn validators(&self, headers: &HeaderMap) -> Validators {
        Validators {
            etag: bounded_header_value(
                headers,
                header::ETAG.as_str(),
                self.config.revalidation.max_validator_length,
            ),
            last_modified: bounded_header_value(
                headers,
                header::LAST_MODIFIED.as_str(),
                self.config.revalidation.max_validator_length,
            ),
        }
    }

    pub fn stored_cache_control(&self, headers: &HeaderMap) -> StoredCacheControl {
        let Some(value) = headers.get(header::CACHE_CONTROL) else {
            return StoredCacheControl::default();
        };
        let Ok(value) = value.to_str() else {
            return StoredCacheControl::default();
        };

        let mut parsed = StoredCacheControl::default();
        for part in value.split(',') {
            let trimmed = part.trim();
            let (name, value) = trimmed
                .split_once('=')
                .map(|(name, value)| (name.trim().to_ascii_lowercase(), Some(value.trim())))
                .unwrap_or_else(|| (trimmed.to_ascii_lowercase(), None));
            match name.as_str() {
                "max-age" => parsed.max_age = value.and_then(parse_delta_seconds),
                "stale-if-error" => parsed.stale_if_error = value.and_then(parse_delta_seconds),
                "no-cache" => parsed.no_cache = true,
                "must-revalidate" => parsed.must_revalidate = true,
                _ => {}
            }
        }
        parsed
    }

    pub fn score(
        &self,
        request: &RequestSignals,
        response: Option<&ResponseSignals>,
        stable_fingerprint: bool,
        high_repeat_rate: bool,
    ) -> i16 {
        let mut score = 0;

        if request.method_cacheable {
            score += 30;
        } else {
            score -= 100;
        }
        if request.has_authorization {
            score -= 100;
        } else {
            score += 20;
        }
        if request.has_cookie {
            score -= 80;
        } else {
            score += 20;
        }
        if request.sensitive_path_score > 0 {
            score -= 30;
        }
        if request.query_param_count <= 2 {
            score += 10;
        } else if request.query_param_count > 8 {
            score -= 40;
        }
        if stable_fingerprint {
            score += 20;
        }
        if high_repeat_rate {
            score += 20;
        }

        if let Some(response) = response {
            if response.has_set_cookie {
                score -= 80;
            } else {
                score += 20;
            }
            match response.cache_control {
                CacheControlClass::NoStore => score -= 100,
                CacheControlClass::Private => score -= 100,
                CacheControlClass::Public
                | CacheControlClass::Absent
                | CacheControlClass::Other => score += 10,
                CacheControlClass::NoCache => score -= 80,
            }
            match response.vary {
                VaryClass::Wildcard | VaryClass::Unsupported(_) => score -= 80,
                VaryClass::Absent | VaryClass::Supported(_) => {}
            }
        }

        score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> PolicyEngine {
        PolicyEngine {
            config: PolicyConfig::default(),
            freshness: FreshnessProfile::Balanced,
        }
    }

    #[test]
    fn authorization_is_protected() {
        let engine = engine();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        let signals = engine.request_signals(&Method::GET, "/api/products", &headers, 0);
        let decision = engine.decide_request(Mode::Auto, RouteState::Auto, &signals, false);
        assert_eq!(decision.decision, Decision::Protect);
        assert!(decision.reasons.contains(&DecisionReason::HasAuthorization));
    }

    #[test]
    fn cookie_is_protected() {
        let engine = engine();
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "session=secret".parse().unwrap());
        let signals = engine.request_signals(&Method::GET, "/api/products", &headers, 0);
        let decision = engine.decide_request(Mode::Auto, RouteState::Auto, &signals, false);
        assert_eq!(decision.decision, Decision::Protect);
        assert!(decision.reasons.contains(&DecisionReason::HasCookie));
    }

    #[test]
    fn cache_control_no_store_blocks_storage() {
        let engine = engine();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            "max-age=30, no-store".parse().unwrap(),
        );
        let signals = engine.response_signals(StatusCode::OK, &headers);
        assert_eq!(signals.cache_control, CacheControlClass::NoStore);
        assert!(!engine.response_is_store_safe(&signals));
    }

    #[test]
    fn vary_wildcard_blocks_reuse() {
        let engine = engine();
        let mut headers = HeaderMap::new();
        headers.insert(header::VARY, "*".parse().unwrap());
        let signals = engine.response_signals(StatusCode::OK, &headers);
        assert_eq!(signals.vary, VaryClass::Wildcard);
        assert!(!engine.response_is_store_safe(&signals));
    }

    #[test]
    fn oversized_response_keeps_explainable_reason() {
        let mut engine = engine();
        engine.config.max_object_size = 4;
        let request = engine.request_signals(&Method::GET, "/api/products", &HeaderMap::new(), 0);
        let response = engine.response_signals(StatusCode::OK, &HeaderMap::new());

        let decision = engine.decide_response(
            Mode::Watch,
            RouteState::Watching,
            &request,
            &response,
            5,
            true,
        );

        assert_eq!(decision.decision, Decision::ObserveOnly);
        assert!(decision.reasons.contains(&DecisionReason::ObjectTooLarge));
        assert!(decision
            .reasons
            .contains(&DecisionReason::InsufficientShadowValidations));
    }
}
