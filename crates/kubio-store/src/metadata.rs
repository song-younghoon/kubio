use bytes::Bytes;
use http::HeaderMap;
use http::{HeaderName, HeaderValue};
use kubio_core::{CacheKeyHash, ResponseFingerprint, RouteId, StoredCacheControl, Validators};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::entry::CacheEntry;
use crate::error::StoreError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DiskEntryMetadata {
    pub(crate) version: u32,
    pub(crate) key: CacheKeyHash,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body_file: String,
    pub(crate) created_at_ms: u64,
    pub(crate) expires_at_ms: u64,
    pub(crate) fresh_until_ms: u64,
    pub(crate) stale_until_ms: Option<u64>,
    pub(crate) validators: Validators,
    pub(crate) cache_control: StoredCacheControl,
    pub(crate) must_revalidate: bool,
    pub(crate) fingerprint: ResponseFingerprint,
    pub(crate) route_id: RouteId,
}

impl DiskEntryMetadata {
    pub(crate) fn from_entry(key: &CacheKeyHash, entry: &CacheEntry) -> Self {
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

pub(crate) fn read_disk_entry(meta_path: &Path) -> Result<(CacheKeyHash, CacheEntry), StoreError> {
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

pub(crate) fn remove_disk_entry_files_for_meta(meta_path: &Path) {
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

pub(crate) fn system_time_to_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn ms_to_system_time(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

pub(crate) fn sync_file(path: &Path) -> Result<(), StoreError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|err| StoreError::Other(format!("sync disk file: {err}")))?;
    file.sync_all()
        .map_err(|err| StoreError::Other(format!("sync disk file: {err}")))
}
