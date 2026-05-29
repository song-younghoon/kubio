# Disk Store

Status: design draft
Target release: `v0.2.0`

## Goals

The disk store should persist safe cache entries across process restarts for local and single-node deployments. It is not a distributed cache and does not provide cross-node coherence.

## User-Facing Config

```yaml
storage:
  kind: "disk"
  path: ".kubio/cache"
  max_size: "1GiB"
  max_object_size: "2MiB"
  sync: false
```

Defaults:

- `kind`: `memory`
- `path`: `.kubio/cache` when kind is `disk`
- `sync`: `false`

`sync: true` may force safer flush behavior at the cost of latency. v0.2.0 can defer strict fsync semantics if documented.

## Store Trait Changes

v0.1.0 store trait:

```rust
async fn get(&self, key: &CacheKeyHash) -> Result<Option<CacheEntry>, StoreError>;
async fn put(&self, key: CacheKeyHash, entry: CacheEntry) -> Result<(), StoreError>;
async fn purge(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError>;
fn stats(&self) -> StoreStats;
```

v0.2.0 may add:

```rust
async fn get_metadata(&self, key: &CacheKeyHash) -> Result<Option<CacheEntryMetadata>, StoreError>;
async fn mark_unusable(&self, key: &CacheKeyHash, reason: StoreInvalidationReason) -> Result<(), StoreError>;
fn kind(&self) -> StoreKind;
```

`get_metadata` avoids cloning large bodies before the proxy knows whether a stale entry can be used.

If this complicates the release, implement disk store behind the existing trait first and optimize later.

## Entry Requirements

Only persist entries that v0.2.0 policy permits:

- Safe request signals.
- Safe response signals.
- Bounded body size.
- Fingerprint available.
- No Set-Cookie.
- No private/no-store.
- Supported Vary.
- Route/key is eligible.

Persisted entry metadata:

- status
- sanitized headers
- body bytes
- created_at
- fresh_until
- stale_until
- validators
- must_revalidate
- fingerprint
- route id
- cache key hash
- stored format version

Do not persist:

- raw request headers
- Authorization values
- Cookie values
- Set-Cookie values
- request bodies
- raw query strings outside hashed key material
- observation event history

## Disk Layout

Preferred implementation: embedded transactional key-value store.

Logical tables:

```text
entries: cache_key_hash -> encoded CacheEntry
route_index: route_hash -> set/cache_key_hash list
metadata: store_version, created_by, limits
```

Encoding should be deterministic and versioned. JSON is easy to inspect but costly for bodies. A binary format such as `bincode` plus explicit version field is acceptable if dependency review passes.

Alternative simple layout:

```text
.kubio/cache/
  manifest.json
  entries/
    ab/
      abcdef1234567890.meta.json
      abcdef1234567890.body
```

If using plain files, writes must be atomic:

1. Write temp body.
2. Write temp metadata.
3. fsync when configured.
4. Rename metadata last.
5. Remove orphan temp files on startup.

## Startup Behavior

On startup:

- Create directory if missing.
- Set owner-only permissions where supported.
- Open store and read format version.
- Reject unsupported newer store versions.
- Opportunistically delete expired entries.
- Recompute stats.

Corruption handling:

- A corrupt individual entry is skipped, evented, and deleted or quarantined.
- A corrupt store header fails startup by default.
- Optional future `storage.fallback_to_memory: true` can downgrade to memory; not required for v0.2.0.

## Eviction

Eviction remains size-based with oldest-first behavior unless a better local policy is implemented.

Required:

- Enforce `max_object_size` before write.
- Enforce `max_size` after write.
- Delete expired entries opportunistically.
- Purge by all, route, and key.
- Stats include entries, bytes, evictions, and store kind.

Optional:

- LRU based on last access.
- Separate stale and fresh eviction priority.

## Concurrency

The proxy hot path must not block the Tokio runtime on disk I/O.

Options:

- Use `spawn_blocking` around blocking store calls.
- Use an async file API where practical.
- Use a dedicated store worker task.

Dashboard reads must not hold locks that block proxy writes for long periods.

## Failure Model

| Failure | Behavior |
| --- | --- |
| Read miss | Treat as cache miss |
| Read error for key | Emit event, pass through to origin |
| Decode error for key | Mark unusable/delete, pass through to origin |
| Write error | Return origin response, emit store error |
| Purge error | Return admin API error |
| Max size reached | Evict before failing write where possible |
| Store unavailable at startup | Fail startup with clear error |

## Metrics

Add or extend:

```text
kubio_cache_entries{store="memory|disk"}
kubio_cache_bytes{store="memory|disk"}
kubio_cache_evictions_total{store="memory|disk",reason="size|expired|corrupt|purge"}
kubio_store_errors_total{store="memory|disk",operation="get|put|purge|startup"}
```

Labels must remain bounded.

## Acceptance

- Disk store persists a safe entry across restart.
- Expired disk entries are not served as fresh after restart.
- Stale disk entries require revalidation or stale-if-error.
- Protected responses are not written to disk.
- Purge all/route/key works for disk.
- Corrupt single entry does not crash the proxy hot path.
- Disk I/O errors return origin responses rather than failed reused responses.
