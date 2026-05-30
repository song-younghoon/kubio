use axum::body::Body;
use axum::http::{Response, StatusCode};
use axum::response::IntoResponse;
use kubio_core::{is_hop_by_hop_header, RouteId};
use kubio_store::CacheEntry;

use crate::alt_svc::{add_alt_svc_header, ALT_SVC_HEADER};
use crate::state::ProxyState;

pub(crate) fn response_from_cache_entry_with_status(
    state: &ProxyState,
    route_id: &RouteId,
    entry: CacheEntry,
    kubio_status: &'static str,
    request_authority: Option<&str>,
) -> Response<Body> {
    let mut builder = Response::builder().status(entry.status);
    for (name, value) in &entry.headers {
        if !is_hop_by_hop_header(name.as_str()) && name.as_str() != ALT_SVC_HEADER {
            builder = builder.header(name, value);
        }
    }
    if state.config.debug_headers {
        builder = builder.header("x-kubio-status", kubio_status);
        let eligibility =
            state
                .observer
                .reuse_eligibility(route_id, &entry.cache_key_hash, true, false);
        builder = builder.header("x-kubio-reuse-source", eligibility.reuse_class.to_string());
        builder = builder.header("x-kubio-reuse-class", eligibility.reuse_class.to_string());
    }
    builder = add_alt_svc_header(builder, state, route_id, request_authority);
    builder
        .body(Body::from(entry.body))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}
