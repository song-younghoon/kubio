//! HTTP reverse proxy runtime for kubio.

mod alt_svc;
mod handler;
mod in_flight;
mod query;
mod route_hints;
mod router;
mod state;

pub use router::{router, run_proxy};
pub use state::ProxyState;
