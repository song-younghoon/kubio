use axum::body::Body;
use axum::http::{header, HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use kubio_core::{is_hop_by_hop_header, should_suppress_response_header_on_hit, RouteId};
use kubio_store::CacheEntry;
use std::time::SystemTime;

use crate::alt_svc::{add_alt_svc_header, ALT_SVC_HEADER};
use crate::runtime::ActiveRuntime;
use crate::state::ProxyState;

pub(crate) fn response_from_cache_entry_with_status(
    state: &ProxyState,
    runtime: &ActiveRuntime,
    route_id: &RouteId,
    entry: CacheEntry,
    kubio_status: &'static str,
    request_authority: Option<&str>,
) -> Response<Body> {
    let mut builder = Response::builder().status(entry.status);
    let route_hint = runtime.route_hints.get(route_id).map(|entry| &entry.hint);
    for (name, value) in &entry.headers {
        let lower = name.as_str().to_ascii_lowercase();
        if !is_hop_by_hop_header(name.as_str())
            && name.as_str() != ALT_SVC_HEADER
            && !should_suppress_response_header_on_hit(
                &runtime.config.policy.response_header_equivalence,
                route_hint.map(|hint| &hint.response_headers),
                &lower,
                &entry.suppressed_response_headers,
            )
        {
            builder = builder.header(name, value);
        }
    }
    if runtime
        .config
        .policy
        .response_header_equivalence
        .serve
        .add_age
    {
        if let Some(age) = age_header_value(&entry) {
            builder = builder.header(header::AGE, age);
        }
    }
    if runtime.config.debug_headers {
        builder = builder.header("x-kubio-status", kubio_status);
        builder = builder.header("x-kubio-config-generation", runtime.generation);
        let eligibility =
            state
                .observer
                .reuse_eligibility(route_id, &entry.cache_key_hash, true, false);
        builder = builder.header("x-kubio-reuse-source", eligibility.reuse_class.to_string());
        builder = builder.header("x-kubio-reuse-class", eligibility.reuse_class.to_string());
        if let Some(route) = state.observer.route_by_hash(&route_id.hash()) {
            builder = builder.header("x-kubio-confidence", route.confidence_tier.to_string());
            let key_shape = if route.query_compacted_groups > 0 {
                "query_compacted"
            } else {
                "exact"
            };
            builder = builder.header("x-kubio-key-shape", key_shape);
        }
        let header_shape = if entry.ignored_response_headers.is_empty() {
            "exact"
        } else {
            "normalized"
        };
        builder = builder.header("x-kubio-header-shape", header_shape);
        if !entry.ignored_response_headers.is_empty() {
            builder = builder.header(
                "x-kubio-response-headers-ignored",
                bounded_header_name_list(&entry.ignored_response_headers),
            );
        }
        if !entry.suppressed_response_headers.is_empty() {
            builder = builder.header(
                "x-kubio-response-headers-suppressed",
                bounded_header_name_list(&entry.suppressed_response_headers),
            );
        }
    }
    builder = add_alt_svc_header(builder, state, runtime, route_id, request_authority);
    builder
        .body(Body::from(entry.body))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn bounded_header_name_list(names: &[String]) -> String {
    const LIMIT: usize = 8;
    let mut sorted = names.to_vec();
    sorted.sort();
    sorted.dedup();
    if sorted.len() > LIMIT {
        let remaining = sorted.len() - LIMIT;
        sorted.truncate(LIMIT);
        sorted.push(format!("+{remaining}"));
    }
    sorted.join(",")
}

fn age_header_value(entry: &CacheEntry) -> Option<HeaderValue> {
    let age = SystemTime::now()
        .duration_since(entry.created_at)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    HeaderValue::from_str(&age.to_string()).ok()
}
