use axum::http::{HeaderMap, HeaderValue};
use http::header;
use kubio_core::{
    is_hop_by_hop_header, should_suppress_response_header_on_hit, EffectiveConfig, RouteHintConfig,
    Validators,
};

pub(crate) fn origin_request_headers(
    headers: &HeaderMap,
    validators: Option<&Validators>,
) -> HeaderMap {
    let mut origin_headers = HeaderMap::new();
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        if name == header::HOST
            || is_hop_by_hop_header_named(name.as_str(), &connection_named_headers)
        {
            continue;
        }
        origin_headers.insert(name.clone(), value.clone());
    }
    if let Some(validators) = validators {
        if let Some(etag) = validators.etag.as_deref() {
            if let Ok(value) = HeaderValue::from_str(etag) {
                origin_headers.insert(header::IF_NONE_MATCH, value);
            }
        }
        if let Some(last_modified) = validators.last_modified.as_deref() {
            if let Ok(value) = HeaderValue::from_str(last_modified) {
                origin_headers.insert(header::IF_MODIFIED_SINCE, value);
            }
        }
    }
    origin_headers
}

pub(crate) fn clone_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut cloned = HeaderMap::new();
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        if !is_hop_by_hop_header_named(name.as_str(), &connection_named_headers) {
            cloned.insert(name.clone(), value.clone());
        }
    }
    cloned
}

pub(crate) fn sanitized_response_headers(
    config: &EffectiveConfig,
    route_hint: Option<&RouteHintConfig>,
    headers: &HeaderMap,
    suppressed_names: &[String],
) -> HeaderMap {
    let mut sanitized = HeaderMap::new();
    let connection_named_headers = connection_header_names(headers);
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if is_hop_by_hop_header_named(&lower, &connection_named_headers)
            || lower == "set-cookie"
            || lower.starts_with("x-kubio-")
            || should_suppress_response_header_on_hit(
                &config.policy.response_header_equivalence,
                route_hint.map(|hint| &hint.response_headers),
                &lower,
                suppressed_names,
            )
        {
            continue;
        }
        sanitized.insert(name.clone(), value.clone());
    }
    sanitized
}

pub(crate) fn declared_request_body_len(headers: &HeaderMap) -> u64 {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

pub(crate) fn header_list_size(headers: &HeaderMap) -> u64 {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() as u64 + value.as_bytes().len() as u64)
        .sum()
}

pub(crate) fn unknown_streaming_body_signal(headers: &HeaderMap) -> u64 {
    if headers.contains_key(header::TRANSFER_ENCODING) {
        1
    } else {
        0
    }
}

pub(crate) fn connection_header_names(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) fn is_hop_by_hop_header_named(name: &str, connection_named_headers: &[String]) -> bool {
    is_hop_by_hop_header(name)
        || connection_named_headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_named_headers_are_removed_from_origin_responses() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, "x-stream-id".parse().unwrap());
        headers.insert("x-stream-id", "abc".parse().unwrap());
        headers.insert("content-type", "text/plain".parse().unwrap());

        let cloned = clone_response_headers(&headers);

        assert!(!cloned.contains_key(header::CONNECTION));
        assert!(!cloned.contains_key("x-stream-id"));
        assert_eq!(cloned.get("content-type").unwrap(), "text/plain");
    }

    #[test]
    fn volatile_response_ids_are_removed_from_stored_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-response-id", "raw-id".parse().unwrap());
        headers.insert("content-type", "text/plain".parse().unwrap());

        let sanitized =
            sanitized_response_headers(&EffectiveConfig::default(), None, &headers, &[]);

        assert!(!sanitized.contains_key("x-response-id"));
        assert_eq!(sanitized.get("content-type").unwrap(), "text/plain");
    }
}
