use crate::query_pattern_matches;

pub fn is_legacy_volatile_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "date"
            | "age"
            | "server"
            | "via"
            | "x-request-id"
            | "traceparent"
            | "tracestate"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

pub fn is_volatile_header(name: &str) -> bool {
    is_default_volatile_response_header(name)
}

pub fn is_default_volatile_response_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "date"
            | "age"
            | "server"
            | "via"
            | "x-request-id"
            | "x-response-id"
            | "x-correlation-id"
            | "x-trace-id"
            | "request-id"
            | "response-id"
            | "correlation-id"
            | "traceparent"
            | "tracestate"
            | "x-amzn-requestid"
            | "x-amzn-trace-id"
            | "x-cloud-trace-context"
            | "x-b3-traceid"
            | "x-b3-spanid"
            | "x-b3-parentspanid"
            | "x-b3-sampled"
            | "x-b3-flags"
            | "cf-ray"
            | "fastly-trace-id"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

pub fn is_response_header_hard_safety_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "set-cookie" | "cache-control" | "vary"
    )
}

pub fn is_response_header_representation_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "content-type"
            | "content-encoding"
            | "content-language"
            | "content-location"
            | "content-range"
            | "location"
            | "link"
    )
}

pub fn is_response_header_validator_or_freshness_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "etag" | "last-modified" | "expires" | "surrogate-control"
    )
}

pub fn is_sensitive_response_header_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "authorization"
            | "cookie"
            | "set-cookie"
            | "proxy-authorization"
            | "www-authenticate"
            | "x-user-id"
            | "x-account-id"
            | "x-session-id"
            | "x-api-key"
    ) || matches_header_pattern("x-auth-*", &lower)
        || matches_header_pattern("x-token-*", &lower)
        || matches_header_pattern("x-csrf-*", &lower)
        || matches_header_pattern("x-feature-*", &lower)
        || matches_header_pattern("x-permission-*", &lower)
        || matches_header_pattern("x-role-*", &lower)
        || matches_header_pattern("x-plan-*", &lower)
        || matches_header_pattern("x-entitlement-*", &lower)
}

pub fn is_response_header_candidate_eligible(name: &str) -> bool {
    !is_default_volatile_response_header(name)
        && !is_response_header_hard_safety_header(name)
        && !is_response_header_representation_header(name)
        && !is_response_header_validator_or_freshness_header(name)
        && !is_sensitive_response_header_name(name)
        && !is_hop_by_hop_header(name)
}

pub fn response_header_pattern_matches(pattern: &str, name: &str) -> bool {
    matches_header_pattern(&pattern.to_ascii_lowercase(), &name.to_ascii_lowercase())
}

fn matches_header_pattern(pattern: &str, name: &str) -> bool {
    query_pattern_matches(pattern, name)
}

pub fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

pub fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
    )
}

pub fn redact_header_value(name: &str, value: &str) -> String {
    if is_sensitive_header(name) {
        "REDACTED".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn redaction_never_returns_sensitive_values(secret in "[A-Za-z0-9]{1,64}") {
            prop_assert_ne!(redact_header_value("authorization", &secret), secret.as_str());
            prop_assert_ne!(redact_header_value("cookie", &secret), secret.as_str());
            prop_assert_ne!(redact_header_value("set-cookie", &secret), secret.as_str());
        }
    }

    #[test]
    fn response_ids_are_default_volatile_headers() {
        assert!(is_default_volatile_response_header("x-response-id"));
        assert!(is_default_volatile_response_header("X-Correlation-Id"));
        assert!(is_default_volatile_response_header("traceparent"));
    }

    #[test]
    fn semantic_headers_are_not_candidate_eligible() {
        assert!(!is_response_header_candidate_eligible("content-type"));
        assert!(!is_response_header_candidate_eligible("etag"));
        assert!(!is_response_header_candidate_eligible("set-cookie"));
        assert!(!is_response_header_candidate_eligible("x-user-id"));
        assert!(is_response_header_candidate_eligible(
            "x-vendor-execution-id"
        ));
    }
}
