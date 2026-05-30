//! HTTP reverse proxy runtime for kubio.

mod alt_svc;
mod cache;
mod handler;
mod headers;
mod in_flight;
mod origin;
mod query;
mod response;
mod revalidation;
mod route_hints;
mod router;
mod state;

pub use router::{router, run_proxy};
pub use state::ProxyState;
