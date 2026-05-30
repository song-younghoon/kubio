use async_trait::async_trait;
use kubio_core::CacheKeyHash;
use serde::{Deserialize, Serialize};

use crate::entry::CacheEntry;
use crate::error::StoreError;
use crate::metrics::StoreStats;
use crate::purge::{PurgeResult, PurgeSelector};

#[async_trait]
pub trait CacheStore: Send + Sync {
    async fn get(&self, key: &CacheKeyHash) -> Result<Option<CacheEntry>, StoreError>;
    async fn put(&self, key: CacheKeyHash, entry: CacheEntry) -> Result<(), StoreError>;
    async fn purge(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError>;
    fn stats(&self) -> StoreStats;
    fn kind(&self) -> StoreKind;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    Memory,
    Disk,
}
