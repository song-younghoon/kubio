# Observability and Dashboard

Status: implemented design reference
Target release: `v0.1.0`

## Goals

kubio must make its automation visible without exposing sensitive data. Operators should be able to answer:

- Is kubio forwarding, protecting, shadow-validating, or reusing traffic?
- Which routes are candidates?
- Why is a route protected?
- Did shadow validation find mismatches?
- What is the estimated and actual origin savings?
- Is the cache within configured memory limits?

## Observation Data Model

Observation state is process-local memory in v0.1.0.

### Route Snapshot

```rust
pub struct RouteSnapshot {
    pub route_id: RouteId,
    pub state: RouteState,
    pub request_count: u64,
    pub origin_count: u64,
    pub reuse_count: u64,
    pub protected_count: u64,
    pub bypass_count: u64,
    pub shadow_matches: u64,
    pub shadow_mismatches: u64,
    pub status_classes: StatusClassCounts,
    pub latency: LatencySnapshot,
    pub repeat_rate: f64,
    pub estimated_savings: f64,
    pub actual_reuse_rate: f64,
    pub score: i16,
    pub reasons: Vec<DecisionReason>,
}
```

### Key Observation

Store by cache key hash, not raw key:

```rust
pub struct KeyObservation {
    pub cache_key_hash: CacheKeyHash,
    pub route_id: RouteId,
    pub seen_count: u64,
    pub last_fingerprint: Option<ResponseFingerprint>,
    pub recent_shadow_matches: u32,
    pub recent_shadow_mismatches: u32,
    pub last_seen_at: SystemTime,
}
```

Key observations must be bounded by count and age.

### Event

```rust
pub struct Event {
    pub timestamp: SystemTime,
    pub event_type: EventType,
    pub route_id: Option<RouteId>,
    pub cache_key_hash: Option<CacheKeyHash>,
    pub reasons: Vec<DecisionReason>,
    pub message: String,
}
```

Event ring buffer default: last 1,000 events.

## Events

Required event types:

- `route_candidate_detected`
- `route_promoted_to_shadow`
- `route_promoted_to_auto`
- `route_demoted_due_to_shadow_mismatch`
- `request_protected_due_to_authorization`
- `request_protected_due_to_cookie`
- `response_not_stored_due_to_no_store`
- `response_not_stored_due_to_private`
- `cache_entry_evicted`
- `store_error_fail_open`
- `panic_switch_enabled`
- `panic_switch_disabled`

Events are for explanation, not durable audit logs.

## Metrics

Endpoint:

```text
GET /metrics
```

The endpoint defaults to `/metrics`; v0.1.0 honors `observability.metrics_path` and can disable metrics with `observability.metrics: false`.

Required metrics:

```text
kubio_requests_total
kubio_origin_requests_total
kubio_reused_responses_total
kubio_protected_requests_total
kubio_bypass_requests_total
kubio_shadow_matches_total
kubio_shadow_mismatches_total
kubio_cache_entries
kubio_cache_bytes
kubio_cache_evictions_total
kubio_request_duration_seconds
kubio_origin_duration_seconds
kubio_policy_decisions_total
```

Allowed labels:

```text
method
route_id
decision
status_class
```

Forbidden labels:

```text
raw path
query string
user id
header value
authorization value
cookie value
ip address by default
```

Route id cardinality can still grow. The observer should cap tracked route ids and group overflow into `__other__` for metrics if needed.

## Dashboard Server

Default bind:

```text
127.0.0.1:9900
```

Pages:

- `/`
- `/routes`
- `/routes/:route_id`
- `/events`
- `/config`

JSON APIs:

- `GET /api/overview`
- `GET /api/routes`
- `GET /api/routes/:route_id`
- `GET /api/events`
- `GET /api/config`
- `POST /api/purge` if admin APIs are enabled

The UI can be server-rendered HTML or a small bundled static app. For v0.1.0, prefer the simplest implementation that is testable and does not introduce a large frontend build requirement.

## Overview API

Response shape:

```json
{
  "mode": "watch",
  "observed_requests": 12481,
  "origin_requests": 12481,
  "reused_responses": 0,
  "protected_requests": 1204,
  "bypassed_requests": 0,
  "candidate_routes": 4,
  "auto_routes": 0,
  "estimated_savings": 0.31,
  "actual_reuse_rate": 0.0,
  "shadow_matches": 0,
  "shadow_mismatches": 0,
  "p50_latency_ms": 34.0,
  "p95_latency_ms": 112.0,
  "cache_entries": 0,
  "cache_bytes": 0
}
```

## Routes API

`GET /api/routes` returns sorted route summaries. Default sort:

1. Auto routes.
2. Candidate routes by estimated savings.
3. Protected routes by request count.
4. Watching routes by request count.

Each row should include:

- route id
- state
- request count
- origin count
- reuse count
- protected count
- shadow matches/mismatches
- estimated savings
- current top reason

## Route Detail API

`GET /api/routes/:route_id` returns:

- route summary
- current state
- explanation
- request count
- repeat rate
- status distribution
- latency distribution
- fingerprint stability
- shadow validation result
- estimated benefit
- current freshness profile
- recent events

Route ids contain spaces and slashes. API should support URL-safe route id encoding or a stable route id hash in path with route id in payload.

Recommendation: use `GET /api/routes/by-hash/:route_hash`.

## Config API

`GET /api/config` returns effective config with redaction:

- origin URL is visible.
- listen addresses are visible.
- admin tokens/secrets are redacted.
- environment-derived secrets are redacted.

Config page is read-only in v0.1.0.

## Purge API

Purge is useful for local development and safety recovery.

Endpoint:

```text
POST /api/purge
```

Body:

```json
{ "selector": "all" }
```

or:

```json
{ "selector": "route", "route_id": "GET /api/products" }
```

Rules:

- Disabled unless admin APIs are enabled.
- If dashboard binds publicly, require admin token.
- Purge affects cache entries, not route observation history.
- Purge events are emitted.

## UI Language

Use product language, not cache jargon, in primary views:

| Internal term | UI term |
| --- | --- |
| cache hit | reused |
| cache miss | sent to origin |
| bypass | passed through |
| not cacheable | protected |
| TTL | freshness |
| invalidation | new data detected |
| fingerprint | response pattern |

## Dashboard Page Design

### Overview

Purpose: show current operating mode and system impact.

Content:

- Mode and origin.
- Observed requests.
- Candidate routes.
- Protected routes.
- Estimated savings.
- Actual reuse rate.
- Shadow matches/mismatches.
- Cache memory usage.
- Recent safety events.

### Routes

Purpose: scan route states and sort by impact.

Content:

- Route table.
- State badges: Watching, Candidate, Auto, Protected, Bypassed.
- Counts and top reasons.
- Filter by state.

### Route Detail

Purpose: explain one route.

Content:

- Route status.
- "kubio's reasoning" generated from decision reasons.
- Traffic and latency stats.
- Shadow validation history.
- Recent route events.

### Events

Purpose: inspect recent decisions and safety changes.

Content:

- Timestamp.
- Event type.
- Route id.
- User-facing reason.

### Config

Purpose: verify effective configuration.

Content:

- Mode.
- Origin.
- Listen addresses.
- Storage limits.
- Policy thresholds.
- Freshness profile.

## Redaction Rules

Dashboard, logs, events, and metrics must never show:

- `Authorization` value.
- `Cookie` value.
- `Set-Cookie` value.
- Request body.
- Raw response body.
- Raw query string by default.

If a route path itself contains sensitive values, route clustering should reduce obvious ids. The dashboard may still show normalized route ids; raw observed paths are not required for v0.1.0.

## Snapshot Performance

Dashboard polling must not block proxy hot path.

Requirements:

- Snapshot generation copies compact data out of observer.
- No long-held global locks during rendering.
- Large event lists are paginated or capped.
- Metrics scrape does not allocate unbounded strings.

## CLI Integration

CLI subcommands can use the same JSON APIs when talking to a running process:

- `kubio routes` -> `GET /api/routes`
- `kubio explain "GET /api/products"` -> route detail lookup
- `kubio purge --all` -> `POST /api/purge`
- `kubio doctor` -> local checks plus optional dashboard/admin reachability

If no running dashboard/admin API is available, commands should fail clearly.
