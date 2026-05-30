# Observability and Dashboard

Status: implemented
Target release: `v0.2.0`

## Goals

v0.2.0 adds more decisions. The dashboard and APIs must make them understandable without exposing sensitive data.

Operators should be able to answer:

- Which responses were revalidated?
- Did origin return 304 or new content?
- Was stale served during an outage?
- Which route hint changed behavior?
- Which query parameters fragment reuse?
- Is the cache memory-backed or disk-backed?
- Did disk storage fail, evict, or skip corrupt entries?

## Observation Model Additions

### Route Snapshot Fields

Add to route snapshots:

```rust
pub struct RouteSnapshot {
    pub revalidation_attempts: u64,
    pub revalidation_not_modified: u64,
    pub revalidation_modified: u64,
    pub revalidation_failed: u64,
    pub stale_served: u64,
    pub stale_denied: u64,
    pub route_hint: Option<String>,
    pub route_hint_applied: u64,
    pub route_hint_rejected: u64,
    pub query_hint_applied: u64,
    pub query_hint_rejected: u64,
    pub query_param_suggestions: u64,
    pub query_params: Vec<QueryParamSnapshot>,
}
```

`route_hint` is the configured display name for the matching hint when a hint has been observed for the route.

### Store Snapshot

```rust
pub struct StoreSnapshot {
    pub kind: StoreKind,
    pub entries: u64,
    pub bytes: u64,
    pub max_size: u64,
    pub max_object_size: u64,
    pub evictions: u64,
    pub disk_path: Option<String>,
    pub startup_recovered_entries: Option<u64>,
    pub corrupt_entries_skipped: Option<u64>,
}
```

`disk_path` may be shown because it is config, not secret. If paths can reveal sensitive usernames in some environments, consider redacting home directories later.

### Query Param Snapshot

```rust
pub struct QueryParamSnapshot {
    pub name: String,
    pub seen_count: u64,
    pub cardinality: String,
    pub fingerprint_sensitive: bool,
    pub configured_action: QueryParamAction,
    pub suggestion: Option<QuerySuggestion>,
}
```

No raw values. Cardinality is derived from bounded value hashes and reports `unknown`, `one`, `low`, `medium`, or `high`.

## Events

Event types are defined for:

- `response_revalidated_not_modified`
- `response_revalidated_modified`
- `response_revalidation_failed`
- `stale_response_served`
- `stale_response_denied`
- `route_hint_applied`
- `route_hint_rejected`
- `query_hint_applied`
- `query_hint_rejected`
- `query_param_suggestion_created`
- `disk_store_opened`
- `disk_store_corrupt_entry_skipped`
- `disk_store_error_fail_open`

Events should include route id and key hash when available, but not validator values, raw query values, or headers.

The implementation emits revalidation, stale, panic-switch, route/query hint, query suggestion, and store fail-open events.

## Metrics

Implemented new metrics:

```text
kubio_revalidation_attempts_total
kubio_revalidation_outcomes_total
kubio_stale_responses_served_total
kubio_stale_responses_denied_total
kubio_cache_entries{store="memory|disk"}
kubio_cache_bytes{store="memory|disk"}
kubio_cache_evictions_total{store="memory|disk"}
kubio_route_hints_applied_total
kubio_route_hints_rejected_total
kubio_query_hints_applied_total
kubio_query_hints_rejected_total
kubio_query_param_suggestions_total
kubio_store_errors_total
```

Allowed labels:

```text
method
route_id
outcome
reason
store
operation
```

Allowed outcome values:

```text
not_modified
modified
failed
skipped
```

Allowed stale denied reasons:

```text
not_allowed
too_old
panic_switch
protected
no_entry
no_validator
```

Forbidden labels remain:

```text
raw path
query string
query value
header value
authorization value
cookie value
ip address by default
disk path
```

## JSON APIs

Existing APIs remain:

- `GET /api/overview`
- `GET /api/routes`
- `GET /api/routes/by-hash/:route_hash`
- `GET /api/events`
- `GET /api/config`
- `POST /api/purge`

Add:

- `GET /api/store`

Query snapshots and hint counters are folded into route detail for v0.2.0. Separate bounded query/hint endpoints remain optional future work if the dashboard needs richer drill-downs.

### Overview Additions

```json
{
  "revalidation_attempts": 91,
  "revalidation_not_modified": 74,
  "revalidation_modified": 12,
  "revalidation_failed": 5,
  "stale_responses_served": 3,
  "route_hints_applied": 42,
  "route_hints_rejected": 1,
  "query_hints_applied": 30,
  "query_hints_rejected": 1,
  "query_param_suggestions": 2,
  "store_errors": 0,
  "store_kind": "disk"
}
```

## Dashboard Pages

### Overview

Add:

- Revalidated responses.
- Stale responses served.
- Hint applied/rejected counters.
- Query suggestion count.
- Store error count.
- Store kind and cache size.
- Recent revalidation/stale events.

### Routes

Add columns:

- Revalidated.
- Stale served.
- Query suggestions count.
- Hint status.

### Route Detail

Add sections:

- Revalidation history.
- Stale-if-error status.
- Query parameters and suggestions.
- Freshness and validator presence.
- Route hints applied.

### Store

New page or config section:

- Store kind.
- Memory/disk usage.
- Disk path if configured.
- Evictions by reason.
- Store errors.
- Startup recovery summary.

## CLI Output

`kubio routes` should include concise fields:

```text
GET /api/products  auto  requests=1000  reused=410  revalidated=80  stale=2
```

`kubio explain "GET /api/products"` should include:

```text
Freshness:
- The route has an ETag validator.
- 74 revalidations returned not modified.
- stale-if-error is enabled for up to 5m.

Query:
- utm_source is ignored by route config.
- gclid is suggested as safe to ignore.
```

## Redaction Rules

Do not expose:

- Validator values by default.
- Raw query values.
- Raw cache keys.
- Request/response body samples.
- Authorization, Cookie, Set-Cookie values.
- Disk file contents.

It is acceptable to expose:

- Header names.
- Query parameter names.
- Validator presence.
- Route hint names.
- Cache key hash.
- Route id.

## Acceptance

- Metrics expose revalidation and stale counters with bounded labels.
- Metrics expose hint and store-error counters with bounded labels.
- Dashboard APIs show store kind, revalidation counts, hint counts, query suggestions, and store errors.
- Query snapshots never include raw values.
- Events explain stale served/denied and hint applied/rejected cases.
- CLI explain includes v0.2.0 revalidation/stale counts without exposing sensitive metadata.
