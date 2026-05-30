use async_trait::async_trait;
use kubio_core::{CacheKeyHash, StorageConfig};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::entry::CacheEntry;
use crate::error::StoreError;
use crate::memory::MemoryStoreInner;
use crate::metadata::{
    read_disk_entry, remove_disk_entry_files_for_meta, sync_file, DiskEntryMetadata,
};
use crate::metrics::{StoreOperation, StoreStats};
use crate::purge::{PurgeResult, PurgeSelector};
use crate::store::{CacheStore, StoreKind};

#[derive(Debug, Clone)]
pub struct DiskStore {
    inner: Arc<Mutex<MemoryStoreInner>>,
    path: PathBuf,
    max_size: u64,
    max_object_size: u64,
    sync: bool,
    startup_recovered_entries: u64,
    corrupt_entries_skipped: u64,
}

impl DiskStore {
    pub fn open(config: &StorageConfig) -> Result<Self, StoreError> {
        let path = config
            .path
            .clone()
            .unwrap_or_else(|| PathBuf::from(".kubio/cache"));
        let entries_dir = path.join("entries");
        std::fs::create_dir_all(&entries_dir)
            .map_err(|err| StoreError::Other(format!("open disk store: {err}")))?;

        let mut inner = MemoryStoreInner::default();
        let mut recovered = 0;
        let mut corrupt = 0;
        for entry in std::fs::read_dir(&entries_dir)
            .map_err(|err| StoreError::Other(format!("read disk store: {err}")))?
        {
            let Ok(entry) = entry else {
                corrupt += 1;
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("tmp") {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            match read_disk_entry(&path) {
                Ok((key, cache_entry)) => {
                    inner.bytes += cache_entry.size_bytes();
                    inner.entries.insert(key, cache_entry);
                    recovered += 1;
                }
                Err(_) => {
                    corrupt += 1;
                    remove_disk_entry_files_for_meta(&path);
                }
            }
        }

        let store = Self {
            inner: Arc::new(Mutex::new(inner)),
            path,
            max_size: config.max_size,
            max_object_size: config.max_object_size,
            sync: config.sync,
            startup_recovered_entries: recovered,
            corrupt_entries_skipped: corrupt,
        };
        {
            let mut inner = store.inner.lock();
            store.purge_expired_locked(&mut inner);
            store.evict_until_within_limit(&mut inner);
        }
        Ok(store)
    }

    fn get_blocking(&self, key: &CacheKeyHash) -> Result<Option<CacheEntry>, StoreError> {
        let mut inner = self.inner.lock();
        self.purge_expired_locked(&mut inner);
        Ok(inner.entries.get(key).cloned())
    }

    fn put_blocking(&self, key: CacheKeyHash, entry: CacheEntry) -> Result<(), StoreError> {
        let entry_size = entry.size_bytes();
        if entry_size > self.max_object_size {
            return Err(StoreError::ObjectTooLarge {
                size: entry_size,
                max: self.max_object_size,
            });
        }

        self.write_entry(&key, &entry)?;
        let mut inner = self.inner.lock();
        if let Some(previous) = inner.entries.insert(key, entry) {
            inner.bytes = inner.bytes.saturating_sub(previous.size_bytes());
        }
        inner.bytes += entry_size;
        self.evict_until_within_limit(&mut inner);
        Ok(())
    }

    fn purge_blocking(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError> {
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
                self.remove_files(&key);
            }
        }

        Ok(PurgeResult {
            purged_entries: purged,
            purged_bytes: bytes,
        })
    }

    fn entries_dir(&self) -> PathBuf {
        self.path.join("entries")
    }

    fn meta_path(&self, key: &CacheKeyHash) -> PathBuf {
        self.entries_dir().join(format!("{}.json", key.0))
    }

    fn body_path(&self, key: &CacheKeyHash) -> PathBuf {
        self.entries_dir().join(format!("{}.body", key.0))
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
                self.remove_files(&oldest_key);
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
                self.remove_files(&key);
            }
        }
    }

    fn remove_files(&self, key: &CacheKeyHash) {
        let _ = std::fs::remove_file(self.meta_path(key));
        let _ = std::fs::remove_file(self.body_path(key));
    }

    fn write_entry(&self, key: &CacheKeyHash, entry: &CacheEntry) -> Result<(), StoreError> {
        let body_path = self.body_path(key);
        let meta_path = self.meta_path(key);
        let body_tmp = body_path.with_extension("body.tmp");
        let meta_tmp = meta_path.with_extension("json.tmp");
        std::fs::write(&body_tmp, &entry.body)
            .map_err(|err| StoreError::Other(format!("write disk body: {err}")))?;
        let metadata = DiskEntryMetadata::from_entry(key, entry);
        let encoded = serde_json::to_vec_pretty(&metadata)
            .map_err(|err| StoreError::Other(format!("encode disk metadata: {err}")))?;
        std::fs::write(&meta_tmp, encoded)
            .map_err(|err| StoreError::Other(format!("write disk metadata: {err}")))?;
        if self.sync {
            sync_file(&body_tmp)?;
            sync_file(&meta_tmp)?;
        }
        std::fs::rename(&body_tmp, &body_path)
            .map_err(|err| StoreError::Other(format!("commit disk body: {err}")))?;
        std::fs::rename(&meta_tmp, &meta_path)
            .map_err(|err| StoreError::Other(format!("commit disk metadata: {err}")))?;
        Ok(())
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
impl CacheStore for DiskStore {
    async fn get(&self, key: &CacheKeyHash) -> Result<Option<CacheEntry>, StoreError> {
        let started = Instant::now();
        let store = self.clone();
        let key = key.clone();
        let result = spawn_disk_task("get", move || store.get_blocking(&key)).await;
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
        let store = self.clone();
        let result = spawn_disk_task("put", move || store.put_blocking(key, entry)).await;
        let saturated = matches!(result, Err(StoreError::ObjectTooLarge { .. }));
        self.record_operation(
            StoreOperation::Put,
            started.elapsed(),
            result.is_ok(),
            saturated,
        );
        result
    }

    async fn purge(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError> {
        let started = Instant::now();
        let store = self.clone();
        let result = spawn_disk_task("purge", move || store.purge_blocking(selector)).await;
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
            kind: StoreKind::Disk,
            disk_path: Some(self.path.display().to_string()),
            startup_recovered_entries: Some(self.startup_recovered_entries),
            corrupt_entries_skipped: Some(self.corrupt_entries_skipped),
            operations: inner.operation_stats.clone(),
        }
    }

    fn kind(&self) -> StoreKind {
        StoreKind::Disk
    }
}

async fn spawn_disk_task<T, F>(operation: &'static str, task: F) -> Result<T, StoreError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, StoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|err| StoreError::Other(format!("disk store {operation} task failed: {err}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http::HeaderMap;
    use kubio_core::{body_hash, ResponseFingerprint, RouteId, StoredCacheControl, Validators};
    use std::time::UNIX_EPOCH;

    use crate::metadata::system_time_to_ms;

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
            route_id: RouteId::new("GET", route),
            cache_key_hash: CacheKeyHash(route.to_string()),
        }
    }

    #[tokio::test]
    async fn disk_store_recovers_entries_after_reopen() {
        let path = temp_store_path();
        let config = StorageConfig {
            kind: "disk".to_string(),
            max_size: 8 * 1024 * 1024,
            max_object_size: 1024 * 1024,
            path: Some(path.clone()),
            sync: false,
        };
        let key = CacheKeyHash("disk-entry".to_string());
        {
            let store = DiskStore::open(&config).unwrap();
            store
                .put(key.clone(), entry("body", "/disk", Duration::from_secs(60)))
                .await
                .unwrap();
        }

        let reopened = DiskStore::open(&config).unwrap();
        let recovered = reopened.get(&key).await.unwrap().unwrap();

        assert_eq!(recovered.body, Bytes::from_static(b"body"));
        assert_eq!(reopened.stats().startup_recovered_entries, Some(1));
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn disk_store_rejects_metadata_body_path_traversal() {
        let path = temp_store_path();
        let entries = path.join("entries");
        std::fs::create_dir_all(&entries).unwrap();
        let now = SystemTime::now();
        let key = CacheKeyHash("evil".to_string());
        let metadata = DiskEntryMetadata {
            version: 1,
            key: key.clone(),
            status: 200,
            headers: Vec::new(),
            body_file: "../outside.body".to_string(),
            created_at_ms: system_time_to_ms(now),
            expires_at_ms: system_time_to_ms(now + Duration::from_secs(60)),
            fresh_until_ms: system_time_to_ms(now + Duration::from_secs(60)),
            stale_until_ms: None,
            validators: Validators::default(),
            cache_control: StoredCacheControl::default(),
            must_revalidate: false,
            fingerprint: ResponseFingerprint::new(200, "h".to_string(), Some("b".to_string())),
            route_id: RouteId::new("GET", "/disk"),
        };
        let meta_path = entries.join("evil.json");
        std::fs::write(&meta_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        std::fs::write(path.join("outside.body"), b"outside").unwrap();

        let store = DiskStore::open(&StorageConfig {
            kind: "disk".to_string(),
            max_size: 8 * 1024 * 1024,
            max_object_size: 1024 * 1024,
            path: Some(path.clone()),
            sync: false,
        })
        .unwrap();

        let stats = store.stats();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.corrupt_entries_skipped, Some(1));
        assert!(!meta_path.exists());
        assert!(path.join("outside.body").exists());
        let _ = std::fs::remove_dir_all(path);
    }

    fn temp_store_path() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "kubio-disk-store-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }
}
