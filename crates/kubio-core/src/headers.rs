pub fn is_volatile_header(name: &str) -> bool {
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
}
