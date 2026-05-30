#[cfg(feature = "experimental-http3")]
use anyhow::Context;
use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use kubio_core::{HttpProtocol, OriginProtocolPreference, RouteId, Validators};
#[cfg(feature = "experimental-http3")]
use kubio_observe::UpstreamHttp3Event;
use kubio_transport::origin_uses_http2_prior_knowledge;
#[cfg(feature = "experimental-http3")]
use kubio_transport::Http3OriginResponse;
use reqwest::Client;
use std::fmt;
use url::Url;

use crate::headers::{declared_request_body_len, origin_request_headers};
use crate::state::ProxyState;

pub(crate) enum OriginResponse {
    Reqwest(reqwest::Response),
    #[cfg(feature = "experimental-http3")]
    Http3(Http3OriginResponse),
}

impl OriginResponse {
    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::Reqwest(response) => response.status(),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(response) => response.status(),
        }
    }

    pub(crate) fn headers(&self) -> &HeaderMap {
        match self {
            Self::Reqwest(response) => response.headers(),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(response) => response.headers(),
        }
    }

    fn protocol(&self) -> HttpProtocol {
        match self {
            Self::Reqwest(response) => http_protocol_from_version(response.version()),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(_) => HttpProtocol::Http3,
        }
    }

    pub(crate) async fn bytes(self) -> Result<bytes::Bytes, OriginError> {
        match self {
            Self::Reqwest(response) => response.bytes().await.map_err(OriginError::Request),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(response) => Ok(response.into_body()),
        }
    }

    pub(crate) fn into_body_stream(self) -> Body {
        match self {
            Self::Reqwest(response) => Body::from_stream(response.bytes_stream()),
            #[cfg(feature = "experimental-http3")]
            Self::Http3(response) => Body::from(response.into_body()),
        }
    }
}

pub(crate) async fn send_origin(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    route_id: &RouteId,
) -> Result<OriginResponse, OriginError> {
    send_origin_with_validators(state, method, uri, headers, body, route_id, None).await
}

pub(crate) async fn send_conditional_origin(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    route_id: &RouteId,
    validators: &Validators,
) -> Result<OriginResponse, OriginError> {
    send_origin_with_validators(
        state,
        method,
        uri,
        headers,
        Body::empty(),
        route_id,
        Some(validators),
    )
    .await
}

async fn send_origin_with_validators(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    route_id: &RouteId,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    #[cfg(feature = "experimental-http3")]
    if origin_http3_attempt_enabled(state) {
        return send_origin_http3_with_fallback(
            state, method, uri, headers, body, route_id, validators,
        )
        .await;
    }

    if origin_protocol_retry_is_possible(state, method, headers) {
        let body = axum::body::to_bytes(body, state.config.policy.max_request_body_size)
            .await
            .map_err(|err| OriginError::BodyRead(err.to_string()))?;
        match send_origin_bytes(
            &state.client,
            state,
            method,
            uri,
            headers,
            body.clone(),
            validators,
        )
        .await
        {
            Ok(response) => return validate_origin_protocol(state, route_id, response),
            Err(OriginError::Request(err)) if origin_protocol_retry_error(&err) => {
                let response = send_origin_bytes(
                    &state.fallback_client,
                    state,
                    method,
                    uri,
                    headers,
                    body,
                    validators,
                )
                .await?;
                return validate_origin_protocol(state, route_id, response);
            }
            Err(err) => return Err(err),
        }
    }

    let response =
        send_origin_stream(&state.client, state, method, uri, headers, body, validators).await?;
    validate_origin_protocol(state, route_id, response)
}

#[cfg(feature = "experimental-http3")]
async fn send_origin_http3_with_fallback(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    route_id: &RouteId,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    if state.config.origin.scheme() != "https" {
        state
            .observer
            .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::SkippedNotHttps);
        return send_origin_after_http3_skip(
            state, method, uri, headers, body, route_id, validators,
        )
        .await;
    }

    let replayable = request_is_replayable_for_protocol_fallback(method, headers);
    if state.config.origin_protocol.fallback && !replayable {
        state.observer.record_upstream_http3_event(
            route_id.clone(),
            UpstreamHttp3Event::SkippedNonReplayable,
        );
        return Err(OriginError::NonReplayableHttp3FallbackBlocked);
    }

    let body = axum::body::to_bytes(body, state.config.policy.max_request_body_size)
        .await
        .map_err(|err| OriginError::BodyRead(err.to_string()))?;
    state
        .observer
        .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Attempt);
    match send_origin_http3_bytes(state, method, uri, headers, body.clone(), validators).await {
        Ok(response) => {
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Success);
            validate_origin_protocol(state, route_id, OriginResponse::Http3(response))
        }
        Err(err) if state.config.origin_protocol.fallback && replayable => {
            warn_origin_http3_fallback(&err);
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Failure);
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Fallback);
            let response = send_origin_bytes(
                &state.fallback_client,
                state,
                method,
                uri,
                headers,
                body,
                validators,
            )
            .await?;
            validate_origin_protocol(state, route_id, response)
        }
        Err(err) => {
            tracing::warn!(error = %err, "required upstream HTTP/3 attempt failed");
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Failure);
            state
                .observer
                .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::RequiredFailure);
            Err(OriginError::Http3RequiredFailed(err.to_string()))
        }
    }
}

#[cfg(feature = "experimental-http3")]
fn warn_origin_http3_fallback(err: &anyhow::Error) {
    tracing::warn!(error = %err, "upstream HTTP/3 attempt failed; falling back");
}

#[cfg(feature = "experimental-http3")]
async fn send_origin_after_http3_skip(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    route_id: &RouteId,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    if state.config.origin_protocol.fallback
        && request_is_replayable_for_protocol_fallback(method, headers)
    {
        let body = axum::body::to_bytes(body, state.config.policy.max_request_body_size)
            .await
            .map_err(|err| OriginError::BodyRead(err.to_string()))?;
        let response = send_origin_bytes(
            &state.fallback_client,
            state,
            method,
            uri,
            headers,
            body,
            validators,
        )
        .await?;
        state
            .observer
            .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::Fallback);
        validate_origin_protocol(state, route_id, response)
    } else {
        if state.config.origin_protocol.fallback {
            state.observer.record_upstream_http3_event(
                route_id.clone(),
                UpstreamHttp3Event::SkippedNonReplayable,
            );
            return Err(OriginError::NonReplayableHttp3FallbackBlocked);
        }
        state
            .observer
            .record_upstream_http3_event(route_id.clone(), UpstreamHttp3Event::RequiredFailure);
        Err(OriginError::Http3RequiredFailed(
            "origin is not HTTPS".to_string(),
        ))
    }
}

#[cfg(feature = "experimental-http3")]
async fn send_origin_http3_bytes(
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: bytes::Bytes,
    validators: Option<&Validators>,
) -> anyhow::Result<Http3OriginResponse> {
    let client = state
        .http3_origin_client
        .as_ref()
        .context("origin HTTP/3 client is not configured")?;
    let url = origin_url(&state.config.origin, uri);
    let headers = origin_request_headers(headers, validators);
    let max_response_body_size = state
        .config
        .performance
        .max_buffered_response_size
        .max(state.config.storage.max_object_size)
        .max(state.config.policy.max_fingerprint_body_size)
        .min(usize::MAX as u64) as usize;
    client
        .send(method, &url, &headers, body, max_response_body_size)
        .await
}

#[cfg(feature = "experimental-http3")]
fn origin_http3_attempt_enabled(state: &ProxyState) -> bool {
    state.config.origin_protocol.http3_experimental
        && state.config.origin_protocol.preferred == OriginProtocolPreference::Http3
}

async fn send_origin_stream(
    client: &Client,
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    let url = origin_url(&state.config.origin, uri);
    let req_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client.request(req_method, url);
    let origin_headers = origin_request_headers(headers, validators);
    for (name, value) in &origin_headers {
        request = request.header(name.as_str(), value.as_bytes());
    }
    request
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
        .map_err(OriginError::Request)
        .map(OriginResponse::Reqwest)
}

async fn send_origin_bytes(
    client: &Client,
    state: &ProxyState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: bytes::Bytes,
    validators: Option<&Validators>,
) -> Result<OriginResponse, OriginError> {
    let url = origin_url(&state.config.origin, uri);
    let req_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client.request(req_method, url);
    let origin_headers = origin_request_headers(headers, validators);
    for (name, value) in &origin_headers {
        request = request.header(name.as_str(), value.as_bytes());
    }
    request
        .body(body)
        .send()
        .await
        .map_err(OriginError::Request)
        .map(OriginResponse::Reqwest)
}

fn validate_origin_protocol(
    state: &ProxyState,
    route_id: &RouteId,
    response: OriginResponse,
) -> Result<OriginResponse, OriginError> {
    let actual_protocol = response.protocol();
    state
        .observer
        .record_upstream_protocol(route_id.clone(), actual_protocol);
    if let Some(expected_protocol) =
        expected_origin_protocol(state.config.origin_protocol.preferred)
    {
        if actual_protocol != expected_protocol {
            if state.config.origin_protocol.fallback {
                state.observer.record_protocol_fallback(
                    route_id.clone(),
                    expected_protocol,
                    actual_protocol,
                );
            } else {
                return Err(OriginError::RequiredProtocol {
                    expected: expected_protocol,
                    actual: actual_protocol,
                });
            }
        }
    }
    Ok(response)
}

fn origin_protocol_retry_is_possible(
    state: &ProxyState,
    method: &Method,
    headers: &HeaderMap,
) -> bool {
    state.config.origin_protocol.fallback
        && origin_uses_http2_prior_knowledge(&state.config)
        && request_is_replayable_for_protocol_fallback(method, headers)
}

fn request_is_replayable_for_protocol_fallback(method: &Method, headers: &HeaderMap) -> bool {
    matches!(method, &Method::GET | &Method::HEAD)
        && declared_request_body_len(headers) == 0
        && !headers.contains_key(http::header::TRANSFER_ENCODING)
}

fn origin_protocol_retry_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_request()
}

#[derive(Debug)]
pub(crate) enum OriginError {
    Request(reqwest::Error),
    BodyRead(String),
    #[cfg(feature = "experimental-http3")]
    Http3RequiredFailed(String),
    #[cfg(feature = "experimental-http3")]
    NonReplayableHttp3FallbackBlocked,
    RequiredProtocol {
        expected: HttpProtocol,
        actual: HttpProtocol,
    },
}

impl OriginError {
    pub(crate) fn is_timeout(&self) -> bool {
        matches!(self, Self::Request(err) if err.is_timeout())
    }
}

impl fmt::Display for OriginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(err) => err.fmt(f),
            Self::BodyRead(err) => write!(f, "origin request body read failed: {err}"),
            #[cfg(feature = "experimental-http3")]
            Self::Http3RequiredFailed(err) => {
                write!(f, "required upstream HTTP/3 failed: {err}")
            }
            #[cfg(feature = "experimental-http3")]
            Self::NonReplayableHttp3FallbackBlocked => {
                f.write_str("upstream HTTP/3 fallback blocked for non-replayable request")
            }
            Self::RequiredProtocol { expected, actual } => {
                write!(
                    f,
                    "origin used {actual} when {expected} was required by origin_protocol"
                )
            }
        }
    }
}

impl std::error::Error for OriginError {}

fn expected_origin_protocol(preferred: OriginProtocolPreference) -> Option<HttpProtocol> {
    match preferred {
        OriginProtocolPreference::Auto => None,
        OriginProtocolPreference::Http1 => Some(HttpProtocol::Http1),
        OriginProtocolPreference::Http2 => Some(HttpProtocol::Http2),
        OriginProtocolPreference::Http3 => Some(HttpProtocol::Http3),
    }
}

pub(crate) fn http_protocol_from_version(version: http::Version) -> HttpProtocol {
    match version {
        http::Version::HTTP_2 => HttpProtocol::Http2,
        http::Version::HTTP_3 => HttpProtocol::Http3,
        _ => HttpProtocol::Http1,
    }
}

pub(crate) fn origin_authority(origin: &Url) -> String {
    let host = origin.host_str().unwrap_or("origin");
    match origin.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

pub(crate) fn origin_url(origin: &Url, uri: &Uri) -> Url {
    let mut url = origin.clone();
    let base_path = origin.path().trim_end_matches('/');
    let request_path = uri.path();
    let path = if base_path.is_empty() || base_path == "/" {
        request_path.to_string()
    } else if request_path == "/" {
        base_path.to_string()
    } else {
        format!("{base_path}{request_path}")
    };
    url.set_path(&path);
    url.set_query(uri.query());
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_url_preserves_request_path_and_query() {
        let origin = Url::parse("http://localhost:3000/base").unwrap();
        let uri: Uri = "/api/products?b=2".parse().unwrap();
        assert_eq!(
            origin_url(&origin, &uri).as_str(),
            "http://localhost:3000/base/api/products?b=2"
        );
    }
}
