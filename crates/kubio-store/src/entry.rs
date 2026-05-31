use bytes::Bytes;
use http::HeaderMap;
use kubio_core::{CacheKeyHash, ResponseFingerprint, RouteId, StoredCacheControl, Validators};
use std::time::SystemTime;

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
    pub ignored_response_headers: Vec<String>,
    pub suppressed_response_headers: Vec<String>,
    pub header_policy_version: u16,
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
