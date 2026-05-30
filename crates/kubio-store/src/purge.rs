use kubio_core::{CacheKeyHash, RouteId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurgeSelector {
    All,
    Route(RouteId),
    Key(CacheKeyHash),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeResult {
    pub purged_entries: u64,
    pub purged_bytes: u64,
}
