//! Local dashboard and admin API.

mod api;
mod auth;
mod html;
mod models;
mod pages;
mod router;
mod state;

pub use models::{OverviewResponse, PurgeRequest};
pub use router::{router, run_dashboard};
pub use state::DashboardState;
