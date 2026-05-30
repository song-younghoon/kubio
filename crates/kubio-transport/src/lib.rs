//! Network transport boundary for kubio proxy runtimes.

mod http12;
mod origin;
mod tls;

#[cfg(feature = "experimental-http3")]
mod http3;

pub const EXPERIMENTAL_HTTP3_FEATURE: &str = "experimental-http3";

pub fn experimental_http3_build_enabled() -> bool {
    cfg!(feature = "experimental-http3")
}

pub use http12::serve_http12_router;
pub use origin::{origin_client_builder, origin_uses_http2_prior_knowledge};

#[cfg(feature = "experimental-http3")]
pub use http3::{
    serve_http3_router, Http3OriginClient, Http3OriginResponse, Http3ServerEvent,
    Http3ServerTelemetry,
};
