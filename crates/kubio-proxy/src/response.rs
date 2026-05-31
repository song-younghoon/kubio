use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use kubio_core::{
    body_hash, stable_header_fingerprint, CacheKeyHash, DecisionReason, EffectiveConfig,
    HeaderFingerprintResult, ResponseFingerprint, RouteHintConfig, RouteId,
};
use kubio_observe::EventType;

use crate::alt_svc::{add_alt_svc_header, ALT_SVC_HEADER};
use crate::headers::{connection_header_names, is_hop_by_hop_header_named};
use crate::state::ProxyState;

#[derive(Debug, Clone)]
pub(crate) struct FingerprintComputation {
    pub(crate) fingerprint: ResponseFingerprint,
    pub(crate) header_result: HeaderFingerprintResult,
}

pub(crate) fn make_fingerprint(
    config: &EffectiveConfig,
    route_hint: Option<&RouteHintConfig>,
    verified_response_header_ignores: &[String],
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
) -> Option<FingerprintComputation> {
    if body.len() as u64 > config.policy.max_fingerprint_body_size {
        return None;
    }
    let header_result = stable_header_fingerprint(
        headers,
        &config.policy.response_header_equivalence,
        route_hint.map(|hint| &hint.response_headers),
        verified_response_header_ignores,
    );
    let fingerprint = ResponseFingerprint::new_with_policy(
        status.as_u16(),
        header_result.hash.clone(),
        Some(body_hash(body)),
        header_result.policy_version,
    );
    Some(FingerprintComputation {
        fingerprint,
        header_result,
    })
}

pub(crate) fn response_from_origin_stream(
    state: &ProxyState,
    route_id: &RouteId,
    status: StatusCode,
    headers: &HeaderMap,
    body: Body,
    request_authority: Option<&str>,
    kubio_status: &'static str,
) -> Response<Body> {
    let mut builder = Response::builder().status(status);
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        if !is_hop_by_hop_header_named(name.as_str(), &connection_named_headers)
            && name.as_str() != ALT_SVC_HEADER
        {
            builder = builder.header(name, value);
        }
    }
    if state.config.debug_headers {
        builder = builder.header("x-kubio-status", kubio_status);
        if let Some(route) = state.observer.route_by_hash(&route_id.hash()) {
            builder = builder.header("x-kubio-reuse-class", route.reuse_class.to_string());
            builder = builder.header("x-kubio-confidence", route.confidence_tier.to_string());
            let key_shape = if route.query_compacted_groups > 0 {
                "query_compacted"
            } else {
                "exact"
            };
            builder = builder.header("x-kubio-key-shape", key_shape);
            if !route.adaptive_blockers.is_empty() {
                let blockers = route
                    .adaptive_blockers
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                builder = builder.header("x-kubio-adaptive-blockers", blockers);
            }
        }
    }
    builder = add_alt_svc_header(builder, state, route_id, request_authority);
    builder
        .body(body)
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

pub(crate) fn should_stream_origin_response(
    state: &ProxyState,
    request_signals: &kubio_policy::RequestSignals,
    response_signals: &kubio_policy::ResponseSignals,
    content_length: Option<u64>,
) -> bool {
    let known_too_large = content_length
        .map(|length| {
            length > state.config.policy.max_fingerprint_body_size
                || length > state.config.storage.max_object_size
                || length > state.config.performance.max_buffered_response_size
        })
        .unwrap_or(false);
    (state.config.performance.stream_unstoreable_bodies
        && (!state.policy.request_is_reuse_safe(request_signals)
            || !state.policy.response_is_store_safe(response_signals)))
        || known_too_large
}

pub(crate) fn record_store_saturation_if_needed(
    state: &ProxyState,
    route_id: &RouteId,
    cache_key_hash: Option<&CacheKeyHash>,
    request_signals: &kubio_policy::RequestSignals,
    response_signals: &kubio_policy::ResponseSignals,
    response_size: Option<u64>,
) {
    let Some(response_size) = response_size else {
        return;
    };
    if response_size <= state.config.storage.max_object_size {
        return;
    }
    if !state.policy.request_is_reuse_safe(request_signals)
        || !state.policy.response_is_store_safe(response_signals)
    {
        return;
    }
    state.observer.push_event(
        EventType::StoreSaturated,
        Some(route_id.clone()),
        cache_key_hash.cloned(),
        vec![DecisionReason::ObjectTooLarge],
        "response was larger than the configured store object limit",
    );
}
