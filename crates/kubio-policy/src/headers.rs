use http::HeaderMap;
use std::time::Duration;

pub(crate) fn bounded_header_value(
    headers: &HeaderMap,
    name: &str,
    max_len: usize,
) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= max_len)
        .map(ToOwned::to_owned)
}

pub(crate) fn parse_delta_seconds(value: &str) -> Option<Duration> {
    value
        .trim_matches('"')
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}
