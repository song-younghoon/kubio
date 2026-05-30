//! Cache store abstractions, memory store, and process-local disk store.

use async_trait::async_trait;
use bytes::Bytes;
use http::HeaderMap;
use http::{HeaderName, HeaderValue};
use kubio_core::{
    CacheKeyHash, ResponseFingerprint, RouteId, StorageConfig, StoredCacheControl, Validators,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[async_trait]
pub trait CacheStore: Send + Sync {
    async fn get(&self, key: &CacheKeyHash) -> Result<Option<CacheEntry>, StoreError>;
    async fn put(&self, key: CacheKeyHash, entry: CacheEntry) -> Result<(), StoreError>;
    async fn purge(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError>;
    fn stats(&self) -> StoreStats;
    fn kind(&self) -> StoreKind;
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
struct MemoryStoreInner {
    entries: HashMap<CacheKeyHash, CacheEntry>,
    bytes: u64,
    evictions: u64,
    operation_stats: StoreOperationMetrics,
}

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

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub created_at: SystemTime,
    pub expires_at: SystemTime,
    pub fresh_until: SystemTime,
    pub stale_until: Option<SystemTime>,
    pub validators: Validators,
    pub cache_control: StoredCacheControl,
    pub must_revalidate: bool,
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
        self.fresh_until > SystemTime::now() && !self.must_revalidate
    }

    pub fn is_stale_usable(&self) -> bool {
        self.expires_at > SystemTime::now()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    Memory,
    Disk,
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
    pub kind: StoreKind,
    pub disk_path: Option<String>,
    pub startup_recovered_entries: Option<u64>,
    pub corrupt_entries_skipped: Option<u64>,
    pub operations: StoreOperationMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreOperation {
    Get,
    Put,
    Purge,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreOperationMetrics {
    pub get: StoreOperationStats,
    pub put: StoreOperationStats,
    pub purge: StoreOperationStats,
    pub saturation_events: u64,
}

impl StoreOperationMetrics {
    fn record(
        &mut self,
        operation: StoreOperation,
        latency: Duration,
        success: bool,
        saturated: bool,
    ) {
        let stats = match operation {
            StoreOperation::Get => &mut self.get,
            StoreOperation::Put => &mut self.put,
            StoreOperation::Purge => &mut self.purge,
        };
        stats.record(latency, success);
        if saturated {
            self.saturation_events += 1;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreOperationStats {
    pub count: u64,
    pub error_count: u64,
    pub total_latency_us: u64,
}

impl StoreOperationStats {
    fn record(&mut self, latency: Duration, success: bool) {
        self.count += 1;
        if !success {
            self.error_count += 1;
        }
        self.total_latency_us = self
            .total_latency_us
            .saturating_add(latency.as_micros().min(u128::from(u64::MAX)) as u64);
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("cache object is too large: {size} > {max}")]
    ObjectTooLarge { size: u64, max: u64 },
    #[error("store error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiskEntryMetadata {
    version: u32,
    key: CacheKeyHash,
    status: u16,
    headers: Vec<(String, String)>,
    body_file: String,
    created_at_ms: u64,
    expires_at_ms: u64,
    fresh_until_ms: u64,
    stale_until_ms: Option<u64>,
    validators: Validators,
    cache_control: StoredCacheControl,
    must_revalidate: bool,
    fingerprint: ResponseFingerprint,
    route_id: RouteId,
}

impl DiskEntryMetadata {
    fn from_entry(key: &CacheKeyHash, entry: &CacheEntry) -> Self {
        Self {
            version: 1,
            key: key.clone(),
            status: entry.status,
            headers: headers_to_disk(&entry.headers),
            body_file: format!("{}.body", key.0),
            created_at_ms: system_time_to_ms(entry.created_at),
            expires_at_ms: system_time_to_ms(entry.expires_at),
            fresh_until_ms: system_time_to_ms(entry.fresh_until),
            stale_until_ms: entry.stale_until.map(system_time_to_ms),
            validators: entry.validators.clone(),
            cache_control: entry.cache_control.clone(),
            must_revalidate: entry.must_revalidate,
            fingerprint: entry.fingerprint.clone(),
            route_id: entry.route_id.clone(),
        }
    }
}

fn read_disk_entry(meta_path: &Path) -> Result<(CacheKeyHash, CacheEntry), StoreError> {
    let metadata = std::fs::read(meta_path)
        .map_err(|err| StoreError::Other(format!("read disk metadata: {err}")))?;
    let metadata: DiskEntryMetadata = serde_json::from_slice(&metadata)
        .map_err(|err| StoreError::Other(format!("decode disk metadata: {err}")))?;
    if metadata.version != 1 {
        return Err(StoreError::Other(
            "unsupported disk entry version".to_string(),
        ));
    }
    let expected_key = meta_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| StoreError::Other("invalid disk metadata file name".to_string()))?;
    if metadata.key.0 != expected_key {
        return Err(StoreError::Other(
            "disk metadata key does not match file name".to_string(),
        ));
    }
    let expected_body_file = format!("{expected_key}.body");
    if metadata.body_file != expected_body_file {
        return Err(StoreError::Other(
            "disk metadata body file does not match cache key".to_string(),
        ));
    }
    let body_path = meta_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(expected_body_file);
    let body = std::fs::read(body_path)
        .map_err(|err| StoreError::Other(format!("read disk body: {err}")))?;
    let headers = headers_from_disk(&metadata.headers)?;
    let key = metadata.key.clone();
    Ok((
        key.clone(),
        CacheEntry {
            status: metadata.status,
            headers,
            body: Bytes::from(body),
            created_at: ms_to_system_time(metadata.created_at_ms),
            expires_at: ms_to_system_time(metadata.expires_at_ms),
            fresh_until: ms_to_system_time(metadata.fresh_until_ms),
            stale_until: metadata.stale_until_ms.map(ms_to_system_time),
            validators: metadata.validators,
            cache_control: metadata.cache_control,
            must_revalidate: metadata.must_revalidate,
            fingerprint: metadata.fingerprint,
            route_id: metadata.route_id,
            cache_key_hash: key,
        },
    ))
}

fn remove_disk_entry_files_for_meta(meta_path: &Path) {
    let _ = std::fs::remove_file(meta_path);
    if let Some(stem) = meta_path.file_stem().and_then(|value| value.to_str()) {
        let _ = std::fs::remove_file(meta_path.with_file_name(format!("{stem}.body")));
        let _ = std::fs::remove_file(meta_path.with_file_name(format!("{stem}.body.tmp")));
        let _ = std::fs::remove_file(meta_path.with_file_name(format!("{stem}.json.tmp")));
    }
}

fn headers_to_disk(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect()
}

fn headers_from_disk(headers: &[(String, String)]) -> Result<HeaderMap, StoreError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_str(name)
            .map_err(|err| StoreError::Other(format!("decode disk header name: {err}")))?;
        let value = HeaderValue::from_str(value)
            .map_err(|err| StoreError::Other(format!("decode disk header value: {err}")))?;
        map.insert(name, value);
    }
    Ok(map)
}

fn system_time_to_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn ms_to_system_time(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn sync_file(path: &Path) -> Result<(), StoreError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|err| StoreError::Other(format!("sync disk file: {err}")))?;
    file.sync_all()
        .map_err(|err| StoreError::Other(format!("sync disk file: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kubio_core::{body_hash, StorageConfig};
    use std::time::{Duration, SystemTime};

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
