# Architecture Delta

Status: implemented baseline and safety-hardened
Target release: `v0.2.0`

## Goals

v0.2.0 adds freshness metadata, conditional revalidation, route hints, query intelligence, and disk storage without replacing the v0.1.0 architecture.

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

Implemented responsibility changes:

- `kubio-core`
  - Add cache metadata types, validator types, route hint config, query hint config, and new decision reasons.
- `kubio-policy`
  - Interpret validators, `no-cache`, origin freshness directives, stale-if-error permission, and route hints.
- `kubio-proxy`
  - Add cache state branching, conditional origin requests, 304 merge handling, stale-if-error fallback, and hint-aware key construction.
- `kubio-observe`
  - Track revalidation outcomes, stale serves, query parameter stats, hint matches, and disk-store events.
- `kubio-store`
  - Extend `CacheEntry`, add disk implementation, expose store kind/stats, and keep metadata-only lookup as a later optimization.
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
```

`fresh_until` controls fresh reuse. `expires_at` remains the store retention boundary and may extend beyond `fresh_until` so entries can be revalidated or used during an explicitly allowed stale-if-error window. A stale entry can exist after `fresh_until`, but it is not reusable unless revalidated or stale-if-error applies.

### Validators

```rust
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}
```

Validator values are response metadata. They may be persisted because entries are stored only after safety policy allows storage. They must still be bounded in length.

### Cache Lookup State

The store returns an optional `CacheEntry`. The proxy derives the logical lookup state from `fresh_until`, `expires_at`, validators, `must_revalidate`, route/key eligibility, panic-switch state, and stale-if-error permission.

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
- Disk path defaults to `.kubio/cache` when `storage.kind: disk`.
- Route paths must be absolute and route match methods must be explicit.
- Hint glob patterns support exact names or trailing `*` only, must be non-empty, and are validated before startup.
- Duplicate route hints for the same method/template fail before startup.
- A `query:` section must define at least one `include` or `ignore` pattern.
- Overlapping query include/ignore patterns fail before startup.
- `stale_if_error.max_stale` must be greater than zero and capped by a global maximum.
- `query_intelligence.auto_ignore` defaults to `false` and is not used to change cache keys in v0.2.0.

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
| Revalidation 304 with unsafe headers | Purge the stored entry and refetch unconditionally |
| Disk store unavailable at startup | Fail startup unless configured fallback is enabled |
| Disk store read error for one entry | Skip entry, emit event, pass through to origin |
| Disk store write error | Return origin response, emit store error |

## Security Boundaries

- Route hints are config data and may be shown in dashboard, except secrets if future fields add them.
- Query stats must store parameter names and bounded classes, not raw sensitive values by default.
- Disk store persists only entries that policy allowed to store.
- Disk metadata body file names must match the cache key and cannot point outside the entry directory.
- Disk store does not claim encryption at rest in v0.2.0.

## Follow-Ups

- Move blocking disk file operations behind `spawn_blocking`, an async file API, or a dedicated store worker before high-concurrency disk-store use.
- Expand query intelligence beyond parameter names and simple noise-parameter suggestions to bounded cardinality and fingerprint sensitivity.
- Add release artifact and Docker smoke automation for the v0.2.0 config.
- Keep dashboard route-hint editing out of v0.2.0; dashboard remains read-only.
