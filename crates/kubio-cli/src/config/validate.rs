use anyhow::{bail, Result};
use kubio_core::{EffectiveConfig, OriginProtocolPreference, RouteHintConfig};
use std::net::IpAddr;

pub(crate) fn validate_config(config: &EffectiveConfig) -> Result<()> {
    if config.origin.scheme() != "http" && config.origin.scheme() != "https" {
        bail!("origin must use http or https");
    }
    if !config.server.protocols.http1 && !config.server.protocols.http2 {
        bail!("at least one of server.protocols.http1 or server.protocols.http2 must be enabled");
    }
    if config.server.protocols.h2c && !config.server.protocols.http2 {
        bail!("server.protocols.h2c requires server.protocols.http2: true");
    }
    if config.server.tls.is_none() && config.server.protocols.http2 && !config.server.protocols.h2c
    {
        bail!("HTTP/2 without TLS requires server.protocols.h2c: true");
    }
    if config.server.http2.max_concurrent_streams == 0 {
        bail!("server.http2.max_concurrent_streams must be greater than zero");
    }
    if config.server.http2.initial_stream_window_size == 0 {
        bail!("server.http2.initial_stream_window_size must be greater than zero");
    }
    if config.server.http2.initial_connection_window_size == 0 {
        bail!("server.http2.initial_connection_window_size must be greater than zero");
    }
    if config.server.http2.max_header_list_size == 0 {
        bail!("server.http2.max_header_list_size must be greater than zero");
    }
    validate_http3_server_config(config)?;
    if config.origin_protocol.preferred == OriginProtocolPreference::Http3
        && !config.origin_protocol.http3_experimental
    {
        bail!("origin_protocol.preferred: http3 requires origin_protocol.http3_experimental: true");
    }
    validate_http3_origin_config(config)?;
    if config.storage.kind != "memory" && config.storage.kind != "disk" {
        bail!("storage.kind must be memory or disk");
    }
    if config.server.origin_timeout.is_zero() {
        bail!("server.origin_timeout_ms must be greater than zero");
    }
    if config.storage.max_size == 0 {
        bail!("storage.max_size must be greater than zero");
    }
    if config.storage.max_object_size == 0 {
        bail!("storage.max_object_size must be greater than zero");
    }
    if config.storage.max_object_size > config.storage.max_size {
        bail!("storage.max_object_size must not exceed storage.max_size");
    }
    if config.policy.max_object_size == 0 {
        bail!("policy.max_object_size must be greater than zero");
    }
    if config.policy.max_fingerprint_body_size == 0 {
        bail!("policy.max_fingerprint_body_size must be greater than zero");
    }
    if config.policy.max_request_body_size == 0 {
        bail!("policy.max_request_body_size must be greater than zero");
    }
    if config.policy.min_route_samples == 0
        || config.policy.min_key_repeats == 0
        || config.policy.min_shadow_validations == 0
    {
        bail!("policy promotion thresholds must be greater than zero");
    }
    if !config.policy.max_shadow_mismatch_rate.is_finite()
        || !(0.0..=1.0).contains(&config.policy.max_shadow_mismatch_rate)
    {
        bail!("policy.max_shadow_mismatch_rate must be between 0 and 1");
    }
    if config.policy.revalidation.max_validator_length == 0 {
        bail!("policy.revalidation.max_validator_length must be greater than zero");
    }
    if config.policy.stale_if_error.max_stale.is_zero() {
        bail!("policy.stale_if_error.max_stale must be greater than zero");
    }
    if config.performance.max_in_flight_requests == 0 {
        bail!("performance.max_in_flight_requests must be greater than zero");
    }
    if config.performance.max_buffered_response_size == 0 {
        bail!("performance.max_buffered_response_size must be greater than zero");
    }
    if config.performance.observer_shards == 0 {
        bail!("performance.observer_shards must be greater than zero");
    }
    if config.performance.origin_pool_max_idle_per_host == 0 {
        bail!("performance.origin_pool_max_idle_per_host must be greater than zero");
    }
    if config.performance.origin_pool_idle_timeout.is_zero() {
        bail!("performance.origin_pool_idle_timeout must be greater than zero");
    }
    validate_route_hints(&config.routes)?;
    if !valid_dashboard_path(&config.observability.metrics_path) {
        bail!("observability.metrics_path must be an absolute dashboard path");
    }
    let dashboard_ip = config.dashboard.listen.ip();
    let public_dashboard = !matches!(dashboard_ip, IpAddr::V4(ip) if ip.is_loopback())
        && !matches!(dashboard_ip, IpAddr::V6(ip) if ip.is_loopback());
    if public_dashboard && !config.dashboard.allow_public {
        bail!("public dashboard binding requires dashboard.allow_public: true");
    }
    if public_dashboard && config.dashboard.admin_api && config.admin_token.is_none() {
        bail!("public dashboard admin API requires admin_token");
    }
    Ok(())
}

fn validate_http3_server_config(config: &EffectiveConfig) -> Result<()> {
    let http3 = &config.server.http3;
    if http3.advertise && !http3.enabled {
        bail!("server.http3.advertise requires server.http3.enabled: true");
    }
    validate_http3_authorities(&http3.authorities)?;
    if http3.max_concurrent_streams == 0 {
        bail!("server.http3.max_concurrent_streams must be greater than zero");
    }
    if http3.max_field_section_size == 0 {
        bail!("server.http3.max_field_section_size must be greater than zero");
    }
    if http3.max_udp_payload_size < 1200 {
        bail!("server.http3.max_udp_payload_size must be at least 1200 bytes");
    }
    if http3.idle_timeout.is_zero() {
        bail!("server.http3.idle_timeout must be greater than zero");
    }
    if http3.advertise {
        if http3.authorities.is_empty() {
            bail!("server.http3.advertise requires at least one server.http3.authorities entry");
        }
        if http3.alt_svc_ma.is_zero() {
            bail!("server.http3.alt_svc_ma must be greater than zero when advertise is enabled");
        }
    }
    if http3.enabled {
        if config.server.tls.is_none() {
            bail!("server.http3.enabled requires server.tls");
        }
        if http3.qpack_max_table_capacity != 0 {
            bail!(
                "server.http3.qpack_max_table_capacity must be 0 with the current HTTP/3 runtime"
            );
        }
        #[cfg(not(feature = "experimental-http3"))]
        bail!("HTTP/3 runtime requires a kubio binary built with --features experimental-http3");
    }
    Ok(())
}

fn validate_http3_origin_config(config: &EffectiveConfig) -> Result<()> {
    if config.origin_protocol.http3_max_idle_connections == 0 {
        bail!("origin_protocol.http3_max_idle_connections must be greater than zero");
    }
    if config.origin_protocol.http3_idle_timeout.is_zero() {
        bail!("origin_protocol.http3_idle_timeout must be greater than zero");
    }
    if config.origin_protocol.http3_experimental {
        #[cfg(not(feature = "experimental-http3"))]
        bail!("upstream HTTP/3 requires a kubio binary built with --features experimental-http3");
        #[cfg(feature = "experimental-http3")]
        {
            if config.origin_protocol.preferred == OriginProtocolPreference::Http3
                && config.origin.scheme() != "https"
                && !config.origin_protocol.fallback
            {
                bail!("origin_protocol.preferred: http3 without fallback requires an https origin");
            }
            for path in &config.origin_protocol.http3_ca_certs {
                if !path.exists() {
                    bail!(
                        "origin_protocol.http3_ca_certs entry does not exist: {}",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_http3_authorities(authorities: &[String]) -> Result<()> {
    for authority in authorities {
        if !valid_http3_authority(authority) {
            bail!(
                "server.http3.authorities entries must be host or host:port values: `{authority}`"
            );
        }
    }
    for (index, authority) in authorities.iter().enumerate() {
        if authorities[index + 1..]
            .iter()
            .any(|other| other == authority)
        {
            bail!("duplicate server.http3.authorities entry `{authority}`");
        }
    }
    Ok(())
}

fn valid_http3_authority(authority: &str) -> bool {
    authority.parse::<http::uri::Authority>().is_ok()
        && !authority.contains("://")
        && !authority.contains('/')
        && !authority.contains('@')
        && !authority
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
}

fn valid_dashboard_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() > 1
        && !path.contains('{')
        && !path.contains('}')
        && !path.contains('*')
        && !path.chars().any(char::is_control)
}

fn validate_route_hints(routes: &[RouteHintConfig]) -> Result<()> {
    let mut seen_routes = Vec::new();
    for route in routes {
        if route.route_match.method.trim().is_empty() {
            bail!("routes[].match.method must not be empty");
        }
        if !route.route_match.path.starts_with('/') {
            bail!("routes[].match.path must be absolute");
        }
        let route_key = (
            route.route_match.method.to_ascii_uppercase(),
            route.route_match.path.clone(),
        );
        if seen_routes.contains(&route_key) {
            bail!(
                "duplicate route hint for {} {}",
                route.route_match.method,
                route.route_match.path
            );
        }
        seen_routes.push(route_key);
        for include in &route.query.include {
            if route
                .query
                .ignore
                .iter()
                .any(|ignore| query_patterns_overlap(include, ignore))
            {
                bail!("query parameter pattern `{include}` conflicts with the route ignore list");
            }
        }
        for pattern in route.query.ignore.iter().chain(route.query.include.iter()) {
            if pattern.is_empty() {
                bail!("query hint patterns must not be empty");
            }
            if pattern.matches('*').count() > 1
                || (pattern.contains('*') && !pattern.ends_with('*'))
            {
                bail!("query hint glob patterns only support a trailing *");
            }
        }
        if route
            .stale_if_error
            .max_stale
            .map(|duration| duration.is_zero())
            .unwrap_or(false)
        {
            bail!("routes[].stale_if_error.max_stale must be greater than zero");
        }
    }
    Ok(())
}

fn query_patterns_overlap(left: &str, right: &str) -> bool {
    match (left.strip_suffix('*'), right.strip_suffix('*')) {
        (Some(left_prefix), Some(right_prefix)) => {
            left_prefix.starts_with(right_prefix) || right_prefix.starts_with(left_prefix)
        }
        (Some(left_prefix), None) => right.starts_with(left_prefix),
        (None, Some(right_prefix)) => left.starts_with(right_prefix),
        (None, None) => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{apply_file_config, FileConfig};
    use kubio_core::{
        RouteFreshnessConfig, RouteMatchConfig, RouteQueryConfig, RouteSafetyConfig,
        RouteStaleIfErrorConfig, RouteVaryConfig, TlsConfig,
    };
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn validation_rejects_invalid_metrics_path() {
        let mut config = EffectiveConfig::default();
        config.observability.metrics_path = "metrics".to_string();

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("metrics_path"));
    }

    #[test]
    fn validation_rejects_http2_without_tls_or_h2c() {
        let mut config = EffectiveConfig::default();
        config.server.protocols.http2 = true;

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("h2c"));
    }

    #[test]
    fn validation_accepts_explicit_h2c() {
        let mut config = EffectiveConfig::default();
        config.server.protocols.http2 = true;
        config.server.protocols.h2c = true;

        validate_config(&config).unwrap();
    }

    #[test]
    fn validation_rejects_http3_without_tls() {
        let mut config = EffectiveConfig::default();
        config.server.http3.enabled = true;

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("server.tls"));
    }

    #[cfg(not(feature = "experimental-http3"))]
    #[test]
    fn validation_rejects_http3_runtime_config() {
        let mut config = EffectiveConfig::default();
        config.server.tls = Some(TlsConfig {
            cert: "cert.pem".into(),
            key: "key.pem".into(),
        });
        config.server.http3.enabled = true;

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("--features experimental-http3"));
    }

    #[cfg(feature = "experimental-http3")]
    #[test]
    fn validation_accepts_http3_runtime_config_with_feature() {
        let mut config = EffectiveConfig::default();
        config.server.tls = Some(TlsConfig {
            cert: "cert.pem".into(),
            key: "key.pem".into(),
        });
        config.server.http3.enabled = true;

        validate_config(&config).unwrap();
    }

    #[cfg(feature = "experimental-http3")]
    #[test]
    fn validation_accepts_http3_alt_svc_config_with_feature() {
        let mut config = EffectiveConfig::default();
        config.server.tls = Some(TlsConfig {
            cert: "cert.pem".into(),
            key: "key.pem".into(),
        });
        config.server.http3.enabled = true;
        config.server.http3.advertise = true;
        config.server.http3.authorities = vec!["api.example.com".to_string()];

        validate_config(&config).unwrap();
    }

    #[test]
    fn validation_rejects_enabled_http3_nonzero_qpack_capacity() {
        let mut config = EffectiveConfig::default();
        config.server.tls = Some(TlsConfig {
            cert: "cert.pem".into(),
            key: "key.pem".into(),
        });
        config.server.http3.enabled = true;
        config.server.http3.qpack_max_table_capacity = 4096;

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("qpack_max_table_capacity"));
    }

    #[test]
    fn validation_rejects_invalid_http3_authority() {
        let mut config = EffectiveConfig::default();
        config.server.http3.authorities = vec!["https://example.com".to_string()];

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("server.http3.authorities"));
    }

    #[test]
    fn http3_file_config_applies_v031_fields() {
        let file: FileConfig = serde_yaml::from_str(
            r#"
origin: "http://localhost:3000"
origin_protocol:
  http3_max_idle_connections: 8
  http3_idle_timeout: "45s"
  http3_ca_certs:
    - "certs/origin-ca.pem"
server:
  http3:
    enabled: false
    listen: "127.0.0.1:8443"
    advertise: false
    authorities:
      - "API.EXAMPLE.COM:443"
    alt_svc_ma: "30m"
    max_concurrent_streams: 64
    max_field_section_size: "32KiB"
    qpack_max_table_capacity: "4KiB"
    max_udp_payload_size: "1400"
    idle_timeout: "15s"
"#,
        )
        .unwrap();
        let mut config = EffectiveConfig::default();

        apply_file_config(&mut config, file).unwrap();

        assert_eq!(
            config.server.http3.listen,
            Some("127.0.0.1:8443".parse().unwrap())
        );
        assert_eq!(config.server.http3.authorities, vec!["api.example.com:443"]);
        assert_eq!(config.server.http3.alt_svc_ma, Duration::from_secs(1800));
        assert_eq!(config.server.http3.max_concurrent_streams, 64);
        assert_eq!(config.server.http3.max_field_section_size, 32 * 1024);
        assert_eq!(config.server.http3.qpack_max_table_capacity, 4 * 1024);
        assert_eq!(config.server.http3.max_udp_payload_size, 1400);
        assert_eq!(config.server.http3.idle_timeout, Duration::from_secs(15));
        assert_eq!(config.origin_protocol.http3_max_idle_connections, 8);
        assert_eq!(
            config.origin_protocol.http3_idle_timeout,
            Duration::from_secs(45)
        );
        assert_eq!(
            config.origin_protocol.http3_ca_certs,
            vec![PathBuf::from("certs/origin-ca.pem")]
        );
    }

    #[cfg(feature = "experimental-http3")]
    #[test]
    fn validation_accepts_upstream_http3_experiment_with_feature() {
        let mut config = EffectiveConfig {
            origin: "https://api.example.com".parse().unwrap(),
            ..Default::default()
        };
        config.origin_protocol.preferred = kubio_core::OriginProtocolPreference::Http3;
        config.origin_protocol.http3_experimental = true;

        validate_config(&config).unwrap();
    }

    #[test]
    fn validation_rejects_zero_promotion_thresholds() {
        let mut config = EffectiveConfig::default();
        config.policy.min_shadow_validations = 0;

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("promotion thresholds"));
    }

    #[test]
    fn validation_rejects_duplicate_route_hints() {
        let config = EffectiveConfig {
            routes: vec![
                test_route_hint("GET", "/api/products"),
                test_route_hint("get", "/api/products"),
            ],
            ..EffectiveConfig::default()
        };

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("duplicate route hint"));
    }

    #[test]
    fn validation_rejects_query_glob_conflicts() {
        let hint = RouteHintConfig {
            query: RouteQueryConfig {
                include: vec!["utm_source".to_string()],
                ignore: vec!["utm_*".to_string()],
            },
            ..test_route_hint("GET", "/api/products")
        };
        let config = EffectiveConfig {
            routes: vec![hint],
            ..EffectiveConfig::default()
        };

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("conflicts"));
    }

    #[test]
    fn route_hint_config_rejects_empty_query_section() {
        let file: FileConfig = serde_yaml::from_str(
            r#"
origin: "http://localhost:3000"
routes:
  - match:
      method: GET
      path: "/api/products"
    query: {}
"#,
        )
        .unwrap();
        let mut config = EffectiveConfig::default();

        let err = apply_file_config(&mut config, file)
            .unwrap_err()
            .to_string();

        assert!(err.contains("routes[].query"));
    }

    fn test_route_hint(method: &str, path: &str) -> RouteHintConfig {
        RouteHintConfig {
            name: None,
            route_match: RouteMatchConfig {
                method: method.to_string(),
                path: path.to_string(),
            },
            freshness: RouteFreshnessConfig::default(),
            query: RouteQueryConfig::default(),
            vary: RouteVaryConfig::default(),
            stale_if_error: RouteStaleIfErrorConfig::default(),
            safety: RouteSafetyConfig::default(),
        }
    }
}
