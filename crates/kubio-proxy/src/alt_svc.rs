use axum::http::{HeaderMap, HeaderValue, Uri};
use http::header;
use kubio_core::{EffectiveConfig, RouteId};
use kubio_observe::{AltSvcOutcome, AltSvcReason};

use crate::runtime::ActiveRuntime;
use crate::state::ProxyState;

pub(crate) const ALT_SVC_HEADER: &str = "alt-svc";

pub(crate) fn request_authority(uri: &Uri, headers: &HeaderMap) -> Option<String> {
    uri.authority()
        .map(|authority| authority.as_str().to_string())
        .or_else(|| {
            headers
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned)
        })
}

pub(crate) fn add_alt_svc_header(
    mut builder: http::response::Builder,
    state: &ProxyState,
    runtime: &ActiveRuntime,
    route_id: &RouteId,
    request_authority: Option<&str>,
) -> http::response::Builder {
    let decision = alt_svc_decision(&runtime.config, request_authority);
    state
        .observer
        .record_alt_svc(route_id.clone(), decision.outcome, decision.reason);
    if let Some(value) = decision.value {
        builder = builder.header(ALT_SVC_HEADER, value);
    }
    builder
}

#[derive(Debug)]
struct AltSvcDecision {
    outcome: AltSvcOutcome,
    reason: AltSvcReason,
    value: Option<HeaderValue>,
}

fn alt_svc_decision(config: &EffectiveConfig, request_authority: Option<&str>) -> AltSvcDecision {
    if !config.server.http3.enabled {
        return AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::Http3Disabled,
            value: None,
        };
    }
    if !config.server.http3.advertise {
        return AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::AdvertiseDisabled,
            value: None,
        };
    }
    let Some(request_authority) = request_authority else {
        return AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::MissingAuthority,
            value: None,
        };
    };
    if !config
        .server
        .http3
        .authorities
        .iter()
        .any(|authority| authority.eq_ignore_ascii_case(request_authority))
    {
        return AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::AuthorityNotAllowed,
            value: None,
        };
    }

    let listen = config.server.http3.listen.unwrap_or(config.server.listen);
    let value = format!(
        "h3=\":{}\"; ma={}",
        listen.port(),
        config.server.http3.alt_svc_ma.as_secs()
    );
    match HeaderValue::from_str(&value) {
        Ok(value) => AltSvcDecision {
            outcome: AltSvcOutcome::Advertised,
            reason: AltSvcReason::ConfiguredAuthority,
            value: Some(value),
        },
        Err(_) => AltSvcDecision {
            outcome: AltSvcOutcome::Skipped,
            reason: AltSvcReason::InvalidValue,
            value: None,
        },
    }
}
