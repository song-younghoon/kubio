use axum::http::{HeaderMap, StatusCode};
use http::header;
use kubio_core::{
    DecisionReason, EffectiveConfig, RouteHintConfig, StaleIfErrorMode, StoredCacheControl,
};
use kubio_store::CacheEntry;
use std::time::{Duration, SystemTime};

use crate::headers::sanitized_response_headers;
use crate::state::ProxyState;

#[derive(Debug, Clone)]
pub(crate) struct EntryFreshness {
    pub(crate) created_at: SystemTime,
    pub(crate) fresh_until: SystemTime,
    pub(crate) stale_until: Option<SystemTime>,
    pub(crate) expires_at: SystemTime,
}

pub(crate) fn entry_freshness(
    state: &ProxyState,
    route_hint: Option<&RouteHintConfig>,
    cache_control: &StoredCacheControl,
    headers: &HeaderMap,
    now: SystemTime,
) -> EntryFreshness {
    let base_ttl = state.policy.freshness_ttl_for_route(route_hint);
    let ttl = cache_control
        .max_age
        .map(|max_age| max_age.min(base_ttl))
        .unwrap_or(base_ttl);
    let must_revalidate = cache_control.no_cache || cache_control.must_revalidate;
    let fresh_until = if must_revalidate { now } else { now + ttl };
    let stale_window = stale_window_from_policy(&state.config, route_hint, cache_control, headers);
    let stale_until = stale_window.map(|window| fresh_until + window);
    EntryFreshness {
        created_at: now,
        fresh_until,
        stale_until,
        expires_at: stale_until
            .unwrap_or(fresh_until + state.config.policy.stale_if_error.max_stale),
    }
}

fn stale_window_from_policy(
    config: &EffectiveConfig,
    route_hint: Option<&RouteHintConfig>,
    cache_control: &StoredCacheControl,
    _headers: &HeaderMap,
) -> Option<Duration> {
    let route_window = route_hint.and_then(|hint| {
        if hint.stale_if_error.enabled {
            Some(
                hint.stale_if_error
                    .max_stale
                    .unwrap_or(config.policy.stale_if_error.max_stale),
            )
        } else {
            None
        }
    });
    let origin_window = cache_control.stale_if_error;
    match config.policy.stale_if_error.mode {
        StaleIfErrorMode::Disabled => route_window,
        StaleIfErrorMode::Origin => route_window.or(origin_window),
        StaleIfErrorMode::Enabled => Some(
            route_window
                .or(origin_window)
                .unwrap_or(config.policy.stale_if_error.max_stale),
        ),
    }
    .map(|window| window.min(config.policy.stale_if_error.max_stale))
}

pub(crate) fn stale_if_error_allowed(
    config: &EffectiveConfig,
    route_hint: Option<&RouteHintConfig>,
    entry: &CacheEntry,
    panic_active: bool,
) -> bool {
    !panic_active
        && entry
            .stale_until
            .map(|until| until > SystemTime::now())
            .unwrap_or(false)
        && (entry.cache_control.stale_if_error.is_some()
            || route_hint
                .map(|hint| hint.stale_if_error.enabled)
                .unwrap_or(false)
            || config.policy.stale_if_error.mode == StaleIfErrorMode::Enabled)
}

pub(crate) fn stale_denial_reason(entry: &CacheEntry) -> DecisionReason {
    if entry
        .stale_until
        .map(|until| until <= SystemTime::now())
        .unwrap_or(true)
    {
        DecisionReason::StaleTooOld
    } else {
        DecisionReason::StaleIfErrorNotAllowed
    }
}

pub(crate) fn refresh_entry_after_304(
    state: &ProxyState,
    route_hint: Option<&RouteHintConfig>,
    mut entry: CacheEntry,
    headers: &HeaderMap,
) -> CacheEntry {
    let sanitized = sanitized_response_headers(
        &state.config,
        route_hint,
        headers,
        &entry.suppressed_response_headers,
    );
    for (name, value) in sanitized {
        if let Some(name) = name {
            if name == header::CONTENT_LENGTH || name == header::TRANSFER_ENCODING {
                continue;
            }
            entry.headers.insert(name, value);
        }
    }
    let cache_control = state.policy.stored_cache_control(&entry.headers);
    let freshness = entry_freshness(
        state,
        route_hint,
        &cache_control,
        &entry.headers,
        SystemTime::now(),
    );
    entry.created_at = freshness.created_at;
    entry.fresh_until = freshness.fresh_until;
    entry.stale_until = freshness.stale_until;
    entry.expires_at = freshness.expires_at;
    entry.validators = state.policy.validators(&entry.headers);
    entry.cache_control = cache_control.clone();
    entry.must_revalidate = cache_control.no_cache || cache_control.must_revalidate;
    entry
}

pub(crate) fn revalidation_metadata_is_safe(state: &ProxyState, headers: &HeaderMap) -> bool {
    let signals = state.policy.response_signals(StatusCode::OK, headers);
    state.policy.response_hard_deny_reasons(&signals).is_empty()
}
