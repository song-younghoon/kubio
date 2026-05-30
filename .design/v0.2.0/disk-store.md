# Disk Store

Status: implemented
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

v0.2.0 adds:

```rust
fn kind(&self) -> StoreKind;
```

Metadata-only lookup and `mark_unusable` are deferred. The implemented v0.2.0 baseline keeps the existing `get`/`put`/`purge` shape and adds `kind()` plus richer stats.

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

Implemented layout:

```text
.kubio/cache/
  entries/
    abcdef1234567890.json
    abcdef1234567890.body
```

Writes use plain files with a versioned JSON metadata file and a separate body file:

1. Write temp body.
2. Write temp metadata.
3. `fsync` both temp files when `storage.sync: true`.
4. Rename body and metadata into place.
5. Remove orphan temp files on startup.

On decode, the metadata key and body file name must match the metadata file stem. This prevents a corrupt metadata file from causing arbitrary body file reads.

## Startup Behavior

On startup:

- Create directory if missing.
- Open store and read entry format versions.
- Reject unsupported newer store versions.
- Opportunistically delete expired entries.
- Skip corrupt entries and remove their metadata/body files.
- Remove orphan temp files.
- Recompute stats.

Corruption handling:

- A corrupt individual entry is skipped and deleted.
- Unsupported entry versions are treated as corrupt entries.
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

The implemented disk store wraps `get`, `put`, and `purge` filesystem work in `spawn_blocking`, keeping blocking disk operations off the Tokio core scheduler. Store open/recovery and dashboard stats collection may still perform synchronous filesystem work outside the request hot path.

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
kubio_cache_evictions_total{store="memory|disk"}
kubio_store_errors_total{store="memory|disk"}
```

Labels must remain bounded. Store fail-open behavior is visible through bounded events, dashboard/API store stats, and the store error counter.

## Acceptance

- Disk store persists a safe entry across restart.
- Expired disk entries are not served as fresh after restart.
- Stale disk entries require revalidation or stale-if-error.
- Protected responses are not written to disk.
- Purge all/route/key works for disk.
- Corrupt single entry does not crash the proxy hot path.
- Corrupt metadata cannot cause path traversal or arbitrary body file reads.
- Disk I/O errors return origin responses rather than failed reused responses.
- Disk get/put/purge operations do not block the Tokio core scheduler.
