//! Main HTTP reverse proxy request flow.

use crate::alt_svc::request_authority;
use crate::cache::response_from_cache_entry_with_status;
use crate::headers::{
    clone_response_headers, declared_request_body_len, header_list_size,
    sanitized_response_headers, unknown_streaming_body_signal,
};
use crate::in_flight::ObservedInFlightPermit;
use crate::origin::{
    http_protocol_from_version, origin_authority, send_conditional_origin, send_origin,
};
use crate::query::{count_query_params, query_param_records};
use crate::response::{
    make_fingerprint, record_store_saturation_if_needed, response_from_origin_stream,
    should_stream_origin_response,
};
use crate::revalidation::{
    entry_freshness, refresh_entry_after_304, revalidation_metadata_is_safe, stale_denial_reason,
    stale_if_error_allowed,
};
use crate::state::ProxyState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, Response, StatusCode};
use axum::response::IntoResponse;
use kubio_core::{
    build_cache_key_with_query_names, CacheKeyHash, Decision, DecisionReason, HttpProtocol, Mode,
    RouteHintConfig, RouteId,
};
use kubio_observe::{EventType, ObservationRecord, RevalidationOutcome};
use kubio_store::{CacheEntry, PurgeSelector};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use tracing::{debug, warn};

pub(crate) async fn proxy_handler(
    State(state): State<ProxyState>,
    request: Request<Body>,
) -> Response<Body> {
    let started = std::time::Instant::now();
    let downstream_protocol = http_protocol_from_version(request.version());
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(ToOwned::to_owned);
    let route_id = RouteId::from_method_path(&method, &path);
    let headers = request.headers().clone();
    let request_authority = request_authority(&uri, &headers);
    if downstream_protocol == HttpProtocol::Http2
        && header_list_size(&headers) > state.config.server.http2.max_header_list_size
    {
        state
            .observer
            .record_header_limit_rejection(route_id, downstream_protocol);
        return StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE.into_response();
    }
    let Ok(permit) = state.in_flight.clone().try_acquire_owned() else {
        state
            .observer
            .record_backpressure_rejection(route_id, downstream_protocol);
        state.observer.record_in_flight(
            state
                .config
                .performance
                .max_in_flight_requests
                .saturating_sub(state.in_flight.available_permits()),
            state.config.performance.max_in_flight_requests,
        );
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let _permit = ObservedInFlightPermit::new(&state, permit);
    state
        .observer
        .record_downstream_protocol(route_id.clone(), downstream_protocol);
    let route_hint_entry = state.route_hints.get(&route_id);
    let route_hint = route_hint_entry.map(|entry| &entry.hint);
    let panic_active = panic_switch_active(state.config.panic_file.as_deref());
    record_panic_switch_transition(&state, panic_active, &route_id, None);

    let request_body_len = declared_request_body_len(&headers);
    if request_body_len > state.config.policy.max_request_body_size as u64 {
        warn!("request body exceeded proxy body limit");
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let signal_body_len = request_body_len
        .max(unknown_streaming_body_signal(&headers))
        .min(usize::MAX as u64) as usize;

    let mut request_signals =
        state
            .policy
            .request_signals(&method, &path, &headers, signal_body_len);
    if route_hint
        .map(|hint| hint.safety.acknowledge_sensitive_path)
        .unwrap_or(false)
    {
        request_signals.sensitive_path_score = 0;
    }
    request_signals.query_param_count = query
        .as_deref()
        .map(count_query_params)
        .unwrap_or_default()
        .min(u16::MAX as usize) as u16;
    let query_records = if state.config.policy.query_intelligence.enabled {
        query_param_records(query.as_deref(), route_hint)
    } else {
        Vec::new()
    };
    state
        .observer
        .record_query_params(route_id.clone(), query_records.clone());

    let route_state = state.observer.route_state(&route_id);
    let mut request_decision = state.policy.decide_request(
        state.config.mode,
        route_state,
        &request_signals,
        panic_active,
    );
    if route_hint
        .map(|hint| hint.safety.force_protect)
        .unwrap_or(false)
    {
        request_decision = kubio_policy::PolicyDecision::new(
            Decision::Protect,
            vec![DecisionReason::RouteHintApplied],
            kubio_core::RouteState::Protected,
            -100,
        );
    }
    record_hint_observations(
        &state,
        &route_id,
        route_hint,
        &request_signals,
        &request_decision,
    );

    let cache_key_hash = if request_signals.method_cacheable {
        let query_config = route_hint.and_then(|hint| {
            if hint.query.is_empty() {
                None
            } else {
                Some(&hint.query)
            }
        });
        let vary_names = route_hint_entry
            .map(|entry| entry.vary_names.as_slice())
            .unwrap_or_else(|| state.route_hints.default_vary_names());
        Some(
            build_cache_key_with_query_names(
                &method,
                state.config.origin.scheme(),
                &origin_authority(&state.config.origin),
                &path,
                query.as_deref(),
                &headers,
                vary_names.iter().map(String::as_str),
                query_config,
            )
            .hash(),
        )
    } else {
        None
    };

    let mut origin_response_override = None;
    let mut stale_error_candidate: Option<(CacheKeyHash, CacheEntry)> = None;

    if state.config.mode == Mode::Auto
        && request_decision.decision != Decision::Protect
        && !panic_active
    {
        if let Some(key_hash) = cache_key_hash.as_ref() {
            if state.observer.is_auto_eligible(&route_id, key_hash) {
                match state.store.get(key_hash).await {
                    Ok(Some(entry)) if entry.is_fresh() => {
                        debug!(route = %route_id, "serving reused response");
                        state.observer.record_reuse(
                            route_id.clone(),
                            key_hash.clone(),
                            entry.status,
                            started.elapsed(),
                        );
                        return response_from_cache_entry_with_status(
                            &state,
                            &route_id,
                            entry,
                            "hit",
                            request_authority.as_deref(),
                        );
                    }
                    Ok(Some(entry)) if entry.is_stale_usable() => {
                        if state.config.policy.revalidation.enabled && entry.validators.available()
                        {
                            match send_conditional_origin(
                                &state,
                                &method,
                                &uri,
                                &headers,
                                &route_id,
                                &entry.validators,
                            )
                            .await
                            {
                                Ok(response) if response.status() == StatusCode::NOT_MODIFIED => {
                                    let not_modified_headers =
                                        clone_response_headers(response.headers());
                                    if revalidation_metadata_is_safe(&state, &not_modified_headers)
                                    {
                                        let refreshed = refresh_entry_after_304(
                                            &state,
                                            route_hint,
                                            entry,
                                            &not_modified_headers,
                                        );
                                        if let Err(err) = state
                                            .store
                                            .put(key_hash.clone(), refreshed.clone())
                                            .await
                                        {
                                            warn!(error = %err, "cache refresh failed after 304");
                                        }
                                        state.observer.record_revalidation(
                                            route_id.clone(),
                                            Some(key_hash.clone()),
                                            RevalidationOutcome::NotModified,
                                        );
                                        state.observer.record_reuse(
                                            route_id.clone(),
                                            key_hash.clone(),
                                            refreshed.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state,
                                            &route_id,
                                            refreshed,
                                            "revalidated",
                                            request_authority.as_deref(),
                                        );
                                    }
                                    if let Err(err) = state
                                        .store
                                        .purge(PurgeSelector::Key(key_hash.clone()))
                                        .await
                                    {
                                        warn!(
                                            error = %err,
                                            "failed to purge cache entry after unsafe 304 metadata"
                                        );
                                        state.observer.push_event(
                                            EventType::StoreErrorFailOpen,
                                            Some(route_id.clone()),
                                            Some(key_hash.clone()),
                                            vec![DecisionReason::StoreError],
                                            "failed to purge cache entry after unsafe 304 metadata",
                                        );
                                    }
                                    origin_response_override = Some(
                                        send_origin(
                                            &state,
                                            &method,
                                            &uri,
                                            &headers,
                                            Body::empty(),
                                            &route_id,
                                        )
                                        .await
                                        .unwrap_or(response),
                                    );
                                }
                                Ok(response) if response.status().is_server_error() => {
                                    state.observer.record_revalidation(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        RevalidationOutcome::Failed,
                                    );
                                    if stale_if_error_allowed(
                                        &state.config,
                                        route_hint,
                                        &entry,
                                        panic_active,
                                    ) {
                                        state.observer.record_stale(
                                            route_id.clone(),
                                            Some(key_hash.clone()),
                                            true,
                                            DecisionReason::StaleIfErrorAllowed,
                                        );
                                        state.observer.record_reuse(
                                            route_id.clone(),
                                            key_hash.clone(),
                                            entry.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state,
                                            &route_id,
                                            entry,
                                            "stale",
                                            request_authority.as_deref(),
                                        );
                                    }
                                    state.observer.record_stale(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        false,
                                        stale_denial_reason(&entry),
                                    );
                                    origin_response_override = Some(response);
                                }
                                Ok(response) => {
                                    state.observer.record_revalidation(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        RevalidationOutcome::Modified,
                                    );
                                    origin_response_override = Some(response);
                                }
                                Err(err) => {
                                    warn!(error = %err, "origin revalidation failed");
                                    state.observer.record_revalidation(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        RevalidationOutcome::Failed,
                                    );
                                    if stale_if_error_allowed(
                                        &state.config,
                                        route_hint,
                                        &entry,
                                        panic_active,
                                    ) {
                                        state.observer.record_stale(
                                            route_id.clone(),
                                            Some(key_hash.clone()),
                                            true,
                                            DecisionReason::StaleIfErrorAllowed,
                                        );
                                        state.observer.record_reuse(
                                            route_id.clone(),
                                            key_hash.clone(),
                                            entry.status,
                                            started.elapsed(),
                                        );
                                        return response_from_cache_entry_with_status(
                                            &state,
                                            &route_id,
                                            entry,
                                            "stale",
                                            request_authority.as_deref(),
                                        );
                                    }
                                    state.observer.record_stale(
                                        route_id.clone(),
                                        Some(key_hash.clone()),
                                        false,
                                        stale_denial_reason(&entry),
                                    );
                                    let status = if err.is_timeout() {
                                        StatusCode::GATEWAY_TIMEOUT
                                    } else {
                                        StatusCode::BAD_GATEWAY
                                    };
                                    return status.into_response();
                                }
                            }
                        } else {
                            state.observer.record_revalidation(
                                route_id.clone(),
                                Some(key_hash.clone()),
                                RevalidationOutcome::Skipped,
                            );
                            stale_error_candidate = Some((key_hash.clone(), entry));
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!(error = %err, "cache lookup failed; passing through to origin");
                        state.observer.push_event(
                            EventType::StoreErrorFailOpen,
                            Some(route_id.clone()),
                            Some(key_hash.clone()),
                            vec![DecisionReason::StoreError],
                            "cache lookup failed; passed through to origin",
                        );
                    }
                }
            }
        }
    }

    let origin_response = if let Some(response) = origin_response_override {
        response
    } else {
        match send_origin(
            &state,
            &method,
            &uri,
            &headers,
            request.into_body(),
            &route_id,
        )
        .await
        {
            Ok(response) => response,
            Err(err) => {
                warn!(error = %err, "origin request failed");
                let status = if err.is_timeout() {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                };
                if let Some((key_hash, entry)) = stale_error_candidate {
                    if stale_if_error_allowed(&state.config, route_hint, &entry, panic_active) {
                        state.observer.record_stale(
                            route_id.clone(),
                            Some(key_hash.clone()),
                            true,
                            DecisionReason::StaleIfErrorAllowed,
                        );
                        state.observer.record_reuse(
                            route_id.clone(),
                            key_hash,
                            entry.status,
                            started.elapsed(),
                        );
                        return response_from_cache_entry_with_status(
                            &state,
                            &route_id,
                            entry,
                            "stale",
                            request_authority.as_deref(),
                        );
                    }
                    state.observer.record_stale(
                        route_id.clone(),
                        Some(key_hash),
                        false,
                        stale_denial_reason(&entry),
                    );
                }
                state.observer.record(ObservationRecord {
                    route_id,
                    cache_key_hash,
                    decision: Decision::Bypass,
                    reasons: vec![DecisionReason::PolicyError],
                    status: status.as_u16(),
                    latency: started.elapsed(),
                    origin: true,
                    reused: false,
                    protected: request_decision.protected(),
                    bypass: true,
                    fingerprint: None,
                    shadow_eligible: false,
                    score: request_decision.score,
                    mode: state.config.mode,
                });
                state.observer.push_event(
                    EventType::OriginRequestFailed,
                    None,
                    None,
                    vec![DecisionReason::PolicyError],
                    if status == StatusCode::GATEWAY_TIMEOUT {
                        "origin request timed out"
                    } else {
                        "origin request failed"
                    },
                );
                return status.into_response();
            }
        }
    };

    let status = origin_response.status();
    let origin_headers = clone_response_headers(origin_response.headers());
    let response_signals = state.policy.response_signals(status, &origin_headers);
    record_store_saturation_if_needed(
        &state,
        &route_id,
        cache_key_hash.as_ref(),
        &request_signals,
        &response_signals,
        response_signals.content_length,
    );
    if should_stream_origin_response(
        &state,
        &request_signals,
        &response_signals,
        response_signals.content_length,
    ) {
        let body_len = response_signals
            .content_length
            .unwrap_or(0)
            .min(usize::MAX as u64) as usize;
        let response_decision = state.policy.decide_response(
            state.config.mode,
            state.observer.route_state(&route_id),
            &request_signals,
            &response_signals,
            body_len,
            false,
        );
        let protected = request_decision.decision == Decision::Protect
            || response_decision.decision == Decision::Protect;
        let final_decision = if matches!(
            request_decision.decision,
            Decision::Protect | Decision::Bypass
        ) {
            request_decision.decision
        } else {
            response_decision.decision
        };
        let reasons = if matches!(
            request_decision.decision,
            Decision::Protect | Decision::Bypass
        ) {
            request_decision.reasons.clone()
        } else {
            response_decision.reasons.clone()
        };

        state.observer.record(ObservationRecord {
            route_id: route_id.clone(),
            cache_key_hash,
            decision: final_decision,
            reasons,
            status: status.as_u16(),
            latency: started.elapsed(),
            origin: true,
            reused: false,
            protected,
            bypass: request_decision.decision == Decision::Bypass,
            fingerprint: None,
            shadow_eligible: false,
            score: response_decision.score,
            mode: state.config.mode,
        });

        return response_from_origin_stream(
            &state,
            &route_id,
            status,
            &origin_headers,
            origin_response.into_body_stream(),
            request_authority.as_deref(),
            if panic_active {
                "bypass"
            } else if protected {
                "protected"
            } else {
                "miss"
            },
        );
    }

    let response_bytes = match origin_response.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(error = %err, "origin response body read failed");
            state.observer.record(ObservationRecord {
                route_id,
                cache_key_hash,
                decision: Decision::Bypass,
                reasons: vec![DecisionReason::PolicyError],
                status: StatusCode::BAD_GATEWAY.as_u16(),
                latency: started.elapsed(),
                origin: true,
                reused: false,
                protected: request_decision.protected(),
                bypass: true,
                fingerprint: None,
                shadow_eligible: false,
                score: request_decision.score,
                mode: state.config.mode,
            });
            state.observer.push_event(
                EventType::OriginRequestFailed,
                None,
                None,
                vec![DecisionReason::PolicyError],
                "origin response body read failed",
            );
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    record_store_saturation_if_needed(
        &state,
        &route_id,
        cache_key_hash.as_ref(),
        &request_signals,
        &response_signals,
        Some(response_bytes.len() as u64),
    );

    let fingerprint = make_fingerprint(&state.config, status, &origin_headers, &response_bytes);
    if let Some(fingerprint) = fingerprint.as_ref() {
        state
            .observer
            .record_query_fingerprint(route_id.clone(), &query_records, fingerprint);
    }
    let response_decision = state.policy.decide_response(
        state.config.mode,
        state.observer.route_state(&route_id),
        &request_signals,
        &response_signals,
        response_bytes.len(),
        fingerprint.is_some(),
    );

    let protected = request_decision.decision == Decision::Protect
        || response_decision.decision == Decision::Protect;
    let final_decision = if matches!(
        request_decision.decision,
        Decision::Protect | Decision::Bypass
    ) {
        request_decision.decision
    } else {
        response_decision.decision
    };
    let reasons = if matches!(
        request_decision.decision,
        Decision::Protect | Decision::Bypass
    ) {
        request_decision.reasons.clone()
    } else {
        response_decision.reasons.clone()
    };

    let shadow_eligible = !panic_active
        && state.policy.request_is_reuse_safe(&request_signals)
        && state.policy.response_is_store_safe(&response_signals)
        && fingerprint.is_some()
        && response_bytes.len() as u64 <= state.config.policy.max_fingerprint_body_size;

    state.observer.record(ObservationRecord {
        route_id: route_id.clone(),
        cache_key_hash: cache_key_hash.clone(),
        decision: final_decision,
        reasons: reasons.clone(),
        status: status.as_u16(),
        latency: started.elapsed(),
        origin: true,
        reused: false,
        protected,
        bypass: request_decision.decision == Decision::Bypass,
        fingerprint: fingerprint.clone(),
        shadow_eligible,
        score: response_decision.score,
        mode: state.config.mode,
    });

    if state.config.mode == Mode::Auto
        && !panic_active
        && !protected
        && state.policy.response_is_store_safe(&response_signals)
        && response_bytes.len() as u64 <= state.config.storage.max_object_size
    {
        let validators = state.policy.validators(&origin_headers);
        let cache_control = state.policy.stored_cache_control(&origin_headers);
        let validator_required = cache_control.no_cache || cache_control.must_revalidate;
        if validator_required && !validators.available() {
            state.observer.record_revalidation(
                route_id.clone(),
                cache_key_hash.clone(),
                RevalidationOutcome::Skipped,
            );
        } else if let (Some(key_hash), Some(fingerprint)) = (cache_key_hash.clone(), fingerprint) {
            if state.observer.is_auto_eligible(&route_id, &key_hash) {
                let freshness = entry_freshness(
                    &state,
                    route_hint,
                    &cache_control,
                    &origin_headers,
                    SystemTime::now(),
                );
                let entry = CacheEntry {
                    status: status.as_u16(),
                    headers: sanitized_response_headers(&origin_headers),
                    body: response_bytes.clone(),
                    created_at: freshness.created_at,
                    expires_at: freshness.expires_at,
                    fresh_until: freshness.fresh_until,
                    stale_until: freshness.stale_until,
                    validators,
                    cache_control: cache_control.clone(),
                    must_revalidate: cache_control.no_cache || cache_control.must_revalidate,
                    fingerprint,
                    route_id: route_id.clone(),
                    cache_key_hash: key_hash.clone(),
                };
                if let Err(err) = state.store.put(key_hash.clone(), entry).await {
                    warn!(error = %err, "cache store failed; origin response still returned");
                    state.observer.push_event(
                        EventType::StoreErrorFailOpen,
                        Some(route_id.clone()),
                        Some(key_hash),
                        vec![DecisionReason::StoreError],
                        "cache store failed; returned origin response",
                    );
                }
            }
        }
    }

    response_from_origin_stream(
        &state,
        &route_id,
        status,
        &origin_headers,
        Body::from(response_bytes),
        request_authority.as_deref(),
        if panic_active {
            "bypass"
        } else if protected {
            "protected"
        } else {
            "miss"
        },
    )
}

fn record_hint_observations(
    state: &ProxyState,
    route_id: &RouteId,
    route_hint: Option<&RouteHintConfig>,
    request_signals: &kubio_policy::RequestSignals,
    request_decision: &kubio_policy::PolicyDecision,
) {
    let Some(hint) = route_hint else {
        return;
    };

    let rejected_by_hard_deny = request_decision.decision == Decision::Protect
        && !request_decision
            .reasons
            .contains(&DecisionReason::RouteHintApplied);
    state.observer.record_route_hint(
        route_id.clone(),
        hint.display_name(),
        !rejected_by_hard_deny,
        if rejected_by_hard_deny {
            DecisionReason::RouteHintRejected
        } else {
            DecisionReason::RouteHintApplied
        },
    );

    if !hint.query.is_empty() {
        let query_hint_applied = request_signals.method_cacheable && !rejected_by_hard_deny;
        state.observer.record_query_hint(
            route_id.clone(),
            query_hint_applied,
            if query_hint_applied {
                DecisionReason::QueryHintApplied
            } else {
                DecisionReason::QueryHintRejected
            },
        );
    }
}

fn panic_switch_active(path: Option<&Path>) -> bool {
    path.map(|path| path.exists()).unwrap_or(false)
}

fn record_panic_switch_transition(
    state: &ProxyState,
    panic_active: bool,
    route_id: &RouteId,
    cache_key_hash: Option<CacheKeyHash>,
) {
    let was_active = state
        .panic_switch_was_active
        .swap(panic_active, Ordering::Relaxed);

    match (was_active, panic_active) {
        (false, true) => state.observer.push_event(
            EventType::PanicSwitchEnabled,
            Some(route_id.clone()),
            cache_key_hash,
            vec![DecisionReason::PanicSwitchActive],
            "panic switch active; response reuse disabled",
        ),
        (true, false) => state.observer.push_event(
            EventType::PanicSwitchDisabled,
            Some(route_id.clone()),
            cache_key_hash,
            vec![DecisionReason::ReusableAndFresh],
            "panic switch inactive; policy-controlled reuse restored",
        ),
        _ => {}
    }
}
