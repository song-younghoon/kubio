//! Cache store abstractions and the v0.1.0 process-local memory store.

use async_trait::async_trait;
use bytes::Bytes;
use http::HeaderMap;
use kubio_core::{CacheKeyHash, ResponseFingerprint, RouteId, StorageConfig};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;
use thiserror::Error;

#[async_trait]
pub trait CacheStore: Send + Sync {
    async fn get(&self, key: &CacheKeyHash) -> Result<Option<CacheEntry>, StoreError>;
    async fn put(&self, key: CacheKeyHash, entry: CacheEntry) -> Result<(), StoreError>;
    async fn purge(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError>;
    fn stats(&self) -> StoreStats;
}

#[derive(Debug)]
pub struct MemoryStore {
    inner: Mutex<MemoryStoreInner>,
    max_size: u64,
    max_object_size: u64,
}

impl MemoryStore {
    pub fn new(config: &StorageConfig) -> Self {
        Self {
            inner: Mutex::new(MemoryStoreInner::default()),
            max_size: config.max_size,
            max_object_size: config.max_object_size,
        }
    }

    fn evict_until_within_limit(&self, inner: &mut MemoryStoreInner) {
        while inner.bytes > self.max_size {
            let Some(oldest_key) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = inner.entries.remove(&oldest_key) {
                inner.bytes = inner.bytes.saturating_sub(entry.size_bytes());
                inner.evictions += 1;
            }
        }
    }

    fn purge_expired_locked(&self, inner: &mut MemoryStoreInner) {
        let now = SystemTime::now();
        let expired = inner
            .entries
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in expired {
            if let Some(entry) = inner.entries.remove(&key) {
                inner.bytes = inner.bytes.saturating_sub(entry.size_bytes());
                inner.evictions += 1;
            }
        }
    }
}

#[async_trait]
impl CacheStore for MemoryStore {
    async fn get(&self, key: &CacheKeyHash) -> Result<Option<CacheEntry>, StoreError> {
        let mut inner = self.inner.lock();
        self.purge_expired_locked(&mut inner);
        Ok(inner.entries.get(key).cloned())
    }

    async fn put(&self, key: CacheKeyHash, entry: CacheEntry) -> Result<(), StoreError> {
        let entry_size = entry.size_bytes();
        if entry_size > self.max_object_size {
            return Err(StoreError::ObjectTooLarge {
                size: entry_size,
                max: self.max_object_size,
            });
        }

        let mut inner = self.inner.lock();
        if let Some(previous) = inner.entries.insert(key, entry) {
            inner.bytes = inner.bytes.saturating_sub(previous.size_bytes());
        }
        inner.bytes += entry_size;
        self.evict_until_within_limit(&mut inner);
        Ok(())
    }

    async fn purge(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError> {
        let mut inner = self.inner.lock();
        let keys = match selector {
            PurgeSelector::All => inner.entries.keys().cloned().collect::<Vec<_>>(),
            PurgeSelector::Route(route_id) => inner
                .entries
                .iter()
                .filter(|(_, entry)| entry.route_id == route_id)
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>(),
            PurgeSelector::Key(key) => vec![key],
        };

        let mut purged = 0;
        let mut bytes = 0;
        for key in keys {
            if let Some(entry) = inner.entries.remove(&key) {
                purged += 1;
                bytes += entry.size_bytes();
                inner.bytes = inner.bytes.saturating_sub(entry.size_bytes());
            }
        }

        Ok(PurgeResult {
            purged_entries: purged,
            purged_bytes: bytes,
        })
    }

    fn stats(&self) -> StoreStats {
        let mut inner = self.inner.lock();
        self.purge_expired_locked(&mut inner);
        StoreStats {
            entries: inner.entries.len() as u64,
            bytes: inner.bytes,
            evictions: inner.evictions,
            max_size: self.max_size,
            max_object_size: self.max_object_size,
        }
    }
}

#[derive(Debug, Default)]
struct MemoryStoreInner {
    entries: HashMap<CacheKeyHash, CacheEntry>,
    bytes: u64,
    evictions: u64,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub created_at: SystemTime,
    pub expires_at: SystemTime,
    pub fingerprint: ResponseFingerprint,
    pub route_id: RouteId,
    pub cache_key_hash: CacheKeyHash,
}

impl CacheEntry {
    pub fn size_bytes(&self) -> u64 {
        let header_bytes = self
            .headers
            .iter()
            .map(|(name, value)| name.as_str().len() + value.as_bytes().len())
            .sum::<usize>() as u64;
        self.body.len() as u64 + header_bytes
    }

    pub fn is_fresh(&self) -> bool {
        self.expires_at > SystemTime::now()
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreStats {
    pub entries: u64,
    pub bytes: u64,
    pub evictions: u64,
    pub max_size: u64,
    pub max_object_size: u64,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("cache object is too large: {size} > {max}")]
    ObjectTooLarge { size: u64, max: u64 },
    #[error("store error: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use kubio_core::{body_hash, StorageConfig};
    use std::time::{Duration, SystemTime};

    fn entry(body: &'static str, route: &str, ttl: Duration) -> CacheEntry {
        CacheEntry {
            status: 200,
            headers: HeaderMap::new(),
            body: Bytes::from_static(body.as_bytes()),
            created_at: SystemTime::now(),
            expires_at: SystemTime::now() + ttl,
            fingerprint: ResponseFingerprint::new(
                200,
                "h".to_string(),
                Some(body_hash(body.as_bytes())),
            ),
            route_id: RouteId::new("GET", route),
            cache_key_hash: CacheKeyHash(route.to_string()),
        }
    }

    #[tokio::test]
    async fn expired_entries_are_not_returned() {
        let store = MemoryStore::new(&StorageConfig::default());
        let key = CacheKeyHash("a".to_string());
        store
            .put(key.clone(), entry("body", "/a", Duration::from_secs(0)))
            .await
            .unwrap();

        assert!(store.get(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn purge_by_route_removes_matching_entries() {
        let store = MemoryStore::new(&StorageConfig::default());
        let key = CacheKeyHash("a".to_string());
        let route = RouteId::new("GET", "/a");
        store
            .put(key.clone(), entry("body", "/a", Duration::from_secs(60)))
            .await
            .unwrap();

        let result = store.purge(PurgeSelector::Route(route)).await.unwrap();
        assert_eq!(result.purged_entries, 1);
        assert!(store.get(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn purge_all_removes_every_entry() {
        let store = MemoryStore::new(&StorageConfig::default());
        store
            .put(
                CacheKeyHash("a".to_string()),
                entry("body", "/a", Duration::from_secs(60)),
            )
            .await
            .unwrap();
        store
            .put(
                CacheKeyHash("b".to_string()),
                entry("body", "/b", Duration::from_secs(60)),
            )
            .await
            .unwrap();

        let result = store.purge(PurgeSelector::All).await.unwrap();

        assert_eq!(result.purged_entries, 2);
        assert_eq!(store.stats().entries, 0);
    }

    #[tokio::test]
    async fn total_size_limit_evicts_oldest_entries() {
        let config = StorageConfig {
            max_size: 8,
            max_object_size: 8,
            ..StorageConfig::default()
        };
        let store = MemoryStore::new(&config);

        store
            .put(
                CacheKeyHash("a".to_string()),
                entry("1234", "/a", Duration::from_secs(60)),
            )
            .await
            .unwrap();
        store
            .put(
                CacheKeyHash("b".to_string()),
                entry("5678", "/b", Duration::from_secs(60)),
            )
            .await
            .unwrap();
        store
            .put(
                CacheKeyHash("c".to_string()),
                entry("9012", "/c", Duration::from_secs(60)),
            )
            .await
            .unwrap();

        let stats = store.stats();
        assert!(stats.entries <= 2);
        assert!(stats.evictions > 0);
    }

    #[tokio::test]
    async fn object_limit_is_enforced() {
        let config = StorageConfig {
            max_object_size: 1,
            ..StorageConfig::default()
        };
        let store = MemoryStore::new(&config);
        let err = store
            .put(
                CacheKeyHash("a".to_string()),
                entry("body", "/a", Duration::from_secs(60)),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::ObjectTooLarge { .. }));
    }
}
