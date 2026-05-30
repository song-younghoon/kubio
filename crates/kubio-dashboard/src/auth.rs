use axum::http::HeaderMap;
use kubio_core::EffectiveConfig;

pub(crate) fn authorized(config: &EffectiveConfig, headers: &HeaderMap) -> bool {
    if !admin_token_required(config) {
        return true;
    }
    let Some(expected) = config.admin_token.as_deref() else {
        return false;
    };
    headers
        .get("x-kubio-admin-token")
        .and_then(|value| value.to_str().ok())
        .map(|actual| actual == expected)
        .unwrap_or(false)
}

fn admin_token_required(config: &EffectiveConfig) -> bool {
    config.dashboard.allow_public || !config.dashboard.listen.ip().is_loopback()
}
