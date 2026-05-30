use kubio_core::HttpProtocol;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProtocolCounts {
    pub http1: u64,
    pub http2: u64,
    pub http3: u64,
}

impl ProtocolCounts {
    pub(crate) fn increment(&mut self, protocol: HttpProtocol) {
        match protocol {
            HttpProtocol::Http1 => self.http1 += 1,
            HttpProtocol::Http2 => self.http2 += 1,
            HttpProtocol::Http3 => self.http3 += 1,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AltSvcCounts {
    pub advertised: u64,
    pub skipped_http3_disabled: u64,
    pub skipped_advertise_disabled: u64,
    pub skipped_missing_authority: u64,
    pub skipped_authority_not_allowed: u64,
    pub skipped_invalid_value: u64,
}

impl AltSvcCounts {
    pub(crate) fn increment(&mut self, outcome: AltSvcOutcome, reason: AltSvcReason) {
        if outcome == AltSvcOutcome::Advertised {
            self.advertised += 1;
            return;
        }
        match reason {
            AltSvcReason::Http3Disabled => self.skipped_http3_disabled += 1,
            AltSvcReason::AdvertiseDisabled => self.skipped_advertise_disabled += 1,
            AltSvcReason::MissingAuthority => self.skipped_missing_authority += 1,
            AltSvcReason::AuthorityNotAllowed => self.skipped_authority_not_allowed += 1,
            AltSvcReason::InvalidValue => self.skipped_invalid_value += 1,
            AltSvcReason::ConfiguredAuthority => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AltSvcOutcome {
    Advertised,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AltSvcReason {
    ConfiguredAuthority,
    Http3Disabled,
    AdvertiseDisabled,
    MissingAuthority,
    AuthorityNotAllowed,
    InvalidValue,
}

impl AltSvcReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::ConfiguredAuthority => "configured_authority",
            Self::Http3Disabled => "http3_disabled",
            Self::AdvertiseDisabled => "advertise_disabled",
            Self::MissingAuthority => "missing_authority",
            Self::AuthorityNotAllowed => "authority_not_allowed",
            Self::InvalidValue => "invalid_value",
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::ConfiguredAuthority => "Alt-Svc advertised for a configured HTTP/3 authority",
            Self::Http3Disabled => "Alt-Svc skipped because downstream HTTP/3 is disabled",
            Self::AdvertiseDisabled => "Alt-Svc skipped because advertisement is disabled",
            Self::MissingAuthority => "Alt-Svc skipped because the request authority was missing",
            Self::AuthorityNotAllowed => {
                "Alt-Svc skipped because the request authority is not configured"
            }
            Self::InvalidValue => "Alt-Svc skipped because the header value could not be rendered",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Http3ServerCounts {
    pub connections_accepted: u64,
    pub handshake_failures: u64,
    pub streams_accepted: u64,
    pub malformed_requests: u64,
    pub request_body_rejections: u64,
    pub response_write_header_errors: u64,
    pub response_write_body_errors: u64,
    pub response_finish_errors: u64,
}

impl Http3ServerCounts {
    pub(crate) fn increment(&mut self, event: Http3ServerEvent) {
        match event {
            Http3ServerEvent::ConnectionAccepted => self.connections_accepted += 1,
            Http3ServerEvent::HandshakeFailed => self.handshake_failures += 1,
            Http3ServerEvent::StreamAccepted => self.streams_accepted += 1,
            Http3ServerEvent::MalformedRequest => self.malformed_requests += 1,
            Http3ServerEvent::RequestBodyRejected => self.request_body_rejections += 1,
            Http3ServerEvent::ResponseWriteHeadersFailed => self.response_write_header_errors += 1,
            Http3ServerEvent::ResponseWriteBodyFailed => self.response_write_body_errors += 1,
            Http3ServerEvent::ResponseFinishFailed => self.response_finish_errors += 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Http3ServerEvent {
    ConnectionAccepted,
    HandshakeFailed,
    StreamAccepted,
    MalformedRequest,
    RequestBodyRejected,
    ResponseWriteHeadersFailed,
    ResponseWriteBodyFailed,
    ResponseFinishFailed,
}

impl Http3ServerEvent {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::ConnectionAccepted => "HTTP/3 connection accepted",
            Self::HandshakeFailed => "HTTP/3 QUIC handshake failed",
            Self::StreamAccepted => "HTTP/3 request stream accepted",
            Self::MalformedRequest => "HTTP/3 request rejected as malformed",
            Self::RequestBodyRejected => "HTTP/3 request body rejected by configured limit",
            Self::ResponseWriteHeadersFailed => "HTTP/3 response header write failed",
            Self::ResponseWriteBodyFailed => "HTTP/3 response body write failed",
            Self::ResponseFinishFailed => "HTTP/3 response finish failed",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpstreamHttp3Counts {
    pub attempts: u64,
    pub successes: u64,
    pub failures: u64,
    pub fallbacks: u64,
    pub required_failures: u64,
    pub skipped_not_https: u64,
    pub skipped_non_replayable: u64,
}

impl UpstreamHttp3Counts {
    pub(crate) fn increment(&mut self, event: UpstreamHttp3Event) {
        match event {
            UpstreamHttp3Event::Attempt => self.attempts += 1,
            UpstreamHttp3Event::Success => self.successes += 1,
            UpstreamHttp3Event::Failure => self.failures += 1,
            UpstreamHttp3Event::Fallback => self.fallbacks += 1,
            UpstreamHttp3Event::RequiredFailure => self.required_failures += 1,
            UpstreamHttp3Event::SkippedNotHttps => self.skipped_not_https += 1,
            UpstreamHttp3Event::SkippedNonReplayable => self.skipped_non_replayable += 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamHttp3Event {
    Attempt,
    Success,
    Failure,
    Fallback,
    RequiredFailure,
    SkippedNotHttps,
    SkippedNonReplayable,
}

impl UpstreamHttp3Event {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Attempt => "upstream HTTP/3 attempt started",
            Self::Success => "upstream HTTP/3 request succeeded",
            Self::Failure => "upstream HTTP/3 request failed",
            Self::Fallback => "upstream HTTP/3 fell back to a lower HTTP protocol",
            Self::RequiredFailure => "required upstream HTTP/3 failed closed",
            Self::SkippedNotHttps => "upstream HTTP/3 skipped because the origin is not HTTPS",
            Self::SkippedNonReplayable => {
                "upstream HTTP/3 fallback blocked for a non-replayable request"
            }
        }
    }
}
