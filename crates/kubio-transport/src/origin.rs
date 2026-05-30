use kubio_core::{EffectiveConfig, OriginProtocolPreference};
use reqwest::Client;
use std::time::Duration;

pub fn origin_client_builder(config: &EffectiveConfig) -> reqwest::ClientBuilder {
    let mut builder = Client::builder()
        .timeout(config.server.origin_timeout)
        .connect_timeout(config.server.origin_timeout.min(Duration::from_secs(5)))
        .pool_max_idle_per_host(config.performance.origin_pool_max_idle_per_host)
        .pool_idle_timeout(config.performance.origin_pool_idle_timeout)
        .http2_initial_stream_window_size(config.server.http2.initial_stream_window_size)
        .http2_initial_connection_window_size(config.server.http2.initial_connection_window_size)
        .http2_max_header_list_size(
            config
                .server
                .http2
                .max_header_list_size
                .min(u64::from(u32::MAX)) as u32,
        )
        .http2_keep_alive_timeout(config.server.http2.keepalive_timeout)
        .http2_keep_alive_while_idle(true);
    if let Some(interval) = config.server.http2.keepalive_interval {
        builder = builder.http2_keep_alive_interval(interval);
    }
    builder
}

pub fn origin_uses_http2_prior_knowledge(config: &EffectiveConfig) -> bool {
    config.origin_protocol.http2_prior_knowledge
        || (config.origin_protocol.preferred == OriginProtocolPreference::Http2
            && config.origin.scheme() == "http")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_prior_knowledge_follows_explicit_flag_or_http_origin_preference() {
        let mut config = EffectiveConfig::default();
        assert!(!origin_uses_http2_prior_knowledge(&config));

        config.origin_protocol.http2_prior_knowledge = true;
        assert!(origin_uses_http2_prior_knowledge(&config));

        config.origin_protocol.http2_prior_knowledge = false;
        config.origin_protocol.preferred = OriginProtocolPreference::Http2;
        config.origin = "https://example.com".parse().unwrap();
        assert!(!origin_uses_http2_prior_knowledge(&config));

        config.origin = "http://example.com".parse().unwrap();
        assert!(origin_uses_http2_prior_knowledge(&config));
    }
}
