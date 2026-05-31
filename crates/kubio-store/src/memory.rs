use async_trait::async_trait;
use kubio_core::{CacheKeyHash, StorageConfig};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use crate::entry::CacheEntry;
use crate::error::StoreError;
use crate::metrics::{StoreOperation, StoreOperationMetrics, StoreStats};
use crate::purge::{PurgeResult, PurgeSelector};
use crate::store::{CacheStore, StoreKind};

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

    fn record_operation(
        &self,
        operation: StoreOperation,
        latency: Duration,
        success: bool,
        saturated: bool,
    ) {
        let mut inner = self.inner.lock();
        inner
            .operation_stats
            .record(operation, latency, success, saturated);
    }
}

#[async_trait]
impl CacheStore for MemoryStore {
    async fn get(&self, key: &CacheKeyHash) -> Result<Option<CacheEntry>, StoreError> {
        let started = Instant::now();
        let result = {
            let mut inner = self.inner.lock();
            self.purge_expired_locked(&mut inner);
            Ok(inner.entries.get(key).cloned())
        };
        self.record_operation(
            StoreOperation::Get,
            started.elapsed(),
            result.is_ok(),
            false,
        );
        result
    }

    async fn put(&self, key: CacheKeyHash, entry: CacheEntry) -> Result<(), StoreError> {
        let started = Instant::now();
        let entry_size = entry.size_bytes();
        if entry_size > self.max_object_size {
            let result = Err(StoreError::ObjectTooLarge {
                size: entry_size,
                max: self.max_object_size,
            });
            self.record_operation(StoreOperation::Put, started.elapsed(), false, true);
            return result;
        }

        let result = {
            let mut inner = self.inner.lock();
            if let Some(previous) = inner.entries.insert(key, entry) {
                inner.bytes = inner.bytes.saturating_sub(previous.size_bytes());
            }
            inner.bytes += entry_size;
            self.evict_until_within_limit(&mut inner);
            Ok(())
        };
        self.record_operation(
            StoreOperation::Put,
            started.elapsed(),
            result.is_ok(),
            false,
        );
        result
    }

    async fn purge(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError> {
        let started = Instant::now();
        let result = {
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
        };
        self.record_operation(
            StoreOperation::Purge,
            started.elapsed(),
            result.is_ok(),
            false,
        );
        result
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
            kind: StoreKind::Memory,
            disk_path: None,
            startup_recovered_entries: None,
            corrupt_entries_skipped: None,
            operations: inner.operation_stats.clone(),
        }
    }

    fn kind(&self) -> StoreKind {
        StoreKind::Memory
    }
}

#[derive(Debug, Default)]
pub(crate) struct MemoryStoreInner {
    pub(crate) entries: HashMap<CacheKeyHash, CacheEntry>,
    pub(crate) bytes: u64,
    pub(crate) evictions: u64,
    pub(crate) operation_stats: StoreOperationMetrics,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http::HeaderMap;
    use kubio_core::{body_hash, ResponseFingerprint, RouteId, StoredCacheControl, Validators};

    fn entry(body: &'static str, route: &str, ttl: Duration) -> CacheEntry {
        let now = SystemTime::now();
        let fresh_until = now + ttl;
        CacheEntry {
            status: 200,
            headers: HeaderMap::new(),
            body: Bytes::from_static(body.as_bytes()),
            created_at: now,
            expires_at: fresh_until,
            fresh_until,
            stale_until: None,
            validators: Validators::default(),
            cache_control: StoredCacheControl::default(),
            must_revalidate: false,
            fingerprint: ResponseFingerprint::new(
                200,
                "h".to_string(),
                Some(body_hash(body.as_bytes())),
            ),
            ignored_response_headers: Vec::new(),
            suppressed_response_headers: Vec::new(),
            header_policy_version: kubio_core::RESPONSE_HEADER_FINGERPRINT_POLICY_VERSION,
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
        let stats = store.stats();
        assert_eq!(stats.operations.put.count, 1);
        assert_eq!(stats.operations.put.error_count, 1);
        assert_eq!(stats.operations.saturation_events, 1);
    }

    #[tokio::test]
    async fn store_operation_stats_are_recorded() {
        let store = MemoryStore::new(&StorageConfig::default());
        let key = CacheKeyHash("a".to_string());

        store
            .put(key.clone(), entry("body", "/a", Duration::from_secs(60)))
            .await
            .unwrap();
        assert!(store.get(&key).await.unwrap().is_some());
        store.purge(PurgeSelector::Key(key)).await.unwrap();

        let stats = store.stats();
        assert_eq!(stats.operations.put.count, 1);
        assert_eq!(stats.operations.get.count, 1);
        assert_eq!(stats.operations.purge.count, 1);
    }
}
