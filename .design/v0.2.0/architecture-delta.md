# Architecture Delta

Status: design draft
Target release: `v0.2.0`

## Goals

v0.2.0 should add freshness metadata, conditional revalidation, route hints, query intelligence, and disk storage without replacing the v0.1.0 architecture.

The proxy hot path still owns request execution. Policy, observation, and stores remain narrow dependencies.

## Workspace Changes

Existing crates remain:

```text
kubio-cli
kubio-core
kubio-proxy
kubio-policy
kubio-observe
kubio-store
kubio-dashboard
kubio-telemetry
```

Expected responsibility changes:

- `kubio-core`
  - Add cache metadata types, validator types, route hint config, query hint config, and new decision reasons.
- `kubio-policy`
  - Interpret validators, `no-cache`, origin freshness directives, stale-if-error permission, and route hints.
- `kubio-proxy`
  - Add cache state branching, conditional origin requests, 304 merge handling, stale-if-error fallback, and hint-aware key construction.
- `kubio-observe`
  - Track revalidation outcomes, stale serves, query parameter stats, hint matches, and disk-store events.
- `kubio-store`
  - Extend `CacheEntry`, add disk implementation, and expose entry metadata without cloning bodies unnecessarily.
- `kubio-dashboard`
  - Surface freshness, validator, stale, query, hint, and store-kind data.
- `kubio-cli`
  - Parse v0.2.0 config and add admin/status output for revalidation and store state.

## Core Type Additions

### Cache Entry Metadata

```rust
pub struct CacheEntry {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub created_at: SystemTime,
    pub fresh_until: SystemTime,
    pub stale_until: Option<SystemTime>,
    pub validators: Validators,
    pub cache_control: StoredCacheControl,
    pub must_revalidate: bool,
    pub fingerprint: ResponseFingerprint,
    pub route_id: RouteId,
    pub cache_key_hash: CacheKeyHash,
}
```

`expires_at` from v0.1.0 becomes `fresh_until`. A stale entry can exist after `fresh_until`, but it is not reusable unless revalidated or stale-if-error applies.

### Validators

```rust
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}
```

Validator values are response metadata. They may be persisted because entries are stored only after safety policy allows storage. They must still be bounded in length.

### Cache Lookup State

```rust
pub enum CacheLookupState {
    Miss,
    Fresh(CacheEntry),
    RequiresRevalidation(CacheEntry),
    StaleIfErrorCandidate(CacheEntry),
}
```

The store may return an entry that is stale. The proxy decides whether it can be used.

### Decision Reasons

Add stable reasons:

```rust
ConditionalRevalidationRequired
RevalidationNotModified
RevalidationModified
RevalidationFailed
NoValidatorAvailable
NoCacheRequiresRevalidation
StaleIfErrorAllowed
StaleIfErrorNotAllowed
StaleTooOld
RouteHintApplied
RouteHintRejected
QueryHintApplied
QueryHintRejected
DiskStoreUnavailable
DiskStoreCorruptEntry
```

Existing reasons keep their names and semantics.

## Config Model

v0.2.0 extends YAML config:

```yaml
policy:
  revalidation:
    enabled: true
    prefer_etag: true
    max_validator_length: 1024
  stale_if_error:
    mode: "origin" # origin | disabled | enabled
    max_stale: "5m"
  query_intelligence:
    enabled: true
    auto_ignore: false

storage:
  kind: "memory" # memory | disk
  max_size: "256MiB"
  max_object_size: "1MiB"
  path: ".kubio/cache"

routes:
  - match:
      method: GET
      path: "/api/products"
    freshness:
      ttl: "60s"
    query:
      ignore: ["utm_*", "gclid"]
    stale_if_error:
      enabled: true
      max_stale: "5m"
```

Validation rules:

- Unknown storage kinds fail before binding.
- Disk path is required when `storage.kind: disk` unless a platform-specific default is chosen.
- Route paths must be absolute and route match methods must be explicit.
- Hint glob patterns must be bounded and compile before startup.
- `stale_if_error.max_stale` must be greater than zero and capped by a global maximum.
- `query.auto_ignore` defaults to `false`.

## Proxy Flow Changes

### Auto Mode Fresh Hit

```text
request precheck
  -> key with route/query hints
  -> store lookup
  -> fresh entry
  -> serve cached response
```

This matches v0.1.0 behavior.

### Auto Mode Stale With Validators

```text
request precheck
  -> store lookup returns stale entry
  -> entry has ETag or Last-Modified
  -> send conditional request to origin
  -> 304: merge metadata, refresh entry, serve stored body
  -> 200: replace entry, return origin body
  -> error: maybe stale-if-error, otherwise gateway error
```

### Auto Mode `no-cache`

`Cache-Control: no-cache` means:

- May be stored when all other safety rules pass.
- Must be revalidated before every reuse.
- Cannot be served as a fresh hit without revalidation.
- Requires a validator for kubio reuse.

### Watch and Shadow Mode

Watch and shadow modes should record validators and freshness metadata but never serve cached or stale responses. Shadow mode may simulate revalidation decisions for explainability, but client-visible behavior remains origin response.

## Failure Model

| Failure | Required behavior |
| --- | --- |
| Conditional request build failure | Pass through to origin without conditional headers |
| 304 missing usable stored body | Treat as store error and pass through to origin |
| Validator too long or malformed | Do not store validator; entry requires origin on stale |
| Revalidation timeout | Serve stale only if stale-if-error applies |
| Revalidation 5xx | Serve stale only if stale-if-error applies |
| Revalidation 200 with unsafe headers | Return origin response but do not store/reuse |
| Disk store unavailable at startup | Fail startup unless configured fallback is enabled |
| Disk store read error for one entry | Skip entry, emit event, pass through to origin |
| Disk store write error | Return origin response, emit store error |

## Security Boundaries

- Route hints are config data and may be shown in dashboard, except secrets if future fields add them.
- Query stats must store parameter names and bounded classes, not raw sensitive values by default.
- Disk store persists only entries that policy allowed to store.
- Disk store directory permissions should be owner-only on Unix where possible.
- Disk store does not claim encryption at rest in v0.2.0.

## Open Questions

- Whether to use `redb`, `sled`, or a custom file layout for disk storage. The design favors an embedded transactional KV store if dependency review passes.
- Whether revalidation should update route shadow confidence after 304. The conservative answer is yes for metadata, but not as a substitute for response fingerprint validation.
- Whether the dashboard should allow editing route hints. v0.2.0 should stay read-only.
