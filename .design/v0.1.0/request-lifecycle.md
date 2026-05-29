# Request Lifecycle

Status: draft
Target release: `v0.1.0`

## Overview

Every request follows one of three mode-specific flows:

- `watch`: forward to origin, observe safely, never serve cached data.
- `shadow`: forward to origin, compare fingerprints for repeated keys, never serve cached data.
- `auto`: serve cached data only when every safety and freshness gate passes; otherwise forward to origin.

All modes must preserve client-visible origin behavior unless `auto` safely returns a verified fresh cache entry.

## Shared Preprocessing

For every inbound request:

1. Assign a request-local trace id.
2. Capture method, URI, headers, and start time.
3. Normalize route id from method and path.
4. Extract request safety signals:
   - method cacheability
   - has `Authorization`
   - has `Cookie`
   - query parameter count
   - content length
   - content type
   - sensitive path score
5. Generate cache key only if method is GET or HEAD and request precheck allows keying.
6. Check panic switch.
7. Build a preliminary decision context.

Preprocessing must not consume request bodies except for normal origin forwarding.

## Hop-by-Hop Headers

The proxy must remove or handle hop-by-hop headers before forwarding:

- `Connection`
- `Keep-Alive`
- `Proxy-Authenticate`
- `Proxy-Authorization`
- `TE`
- `Trailer`
- `Transfer-Encoding`
- `Upgrade`

If the `Connection` header names additional hop-by-hop headers, remove those as well.

## Watch Mode Flow

```text
client request
  -> request safety precheck
  -> origin request
  -> response safety extraction
  -> fingerprint if bounded and eligible
  -> observation record
  -> client response from origin
```

Rules:

- Never call `CacheStore::get`.
- Do not store response bodies as cache entries.
- Record metadata and fingerprints only when safe and bounded.
- Protected conditions are visible in route state and event stream.
- Client response must be origin response.

Watch mode acceptance:

- No `kubio_reused_responses_total` increments.
- Dashboard can show route counts, protected counts, latency, status classes, and candidate hints.

## Shadow Mode Flow

```text
client request
  -> request safety precheck
  -> origin request
  -> response safety extraction
  -> fingerprint if bounded and eligible
  -> compare with previous fingerprint for same cache key hash
  -> record shadow match or mismatch
  -> update route state
  -> client response from origin
```

Rules:

- Never serve cached data.
- A previous fingerprint is a prediction of what kubio would have reused.
- A match increases confidence.
- A mismatch demotes the route/key and prevents auto promotion.
- Only compare fingerprints for requests/responses that pass hard safety denies.

Shadow mode acceptance:

- Stable repeated GET/HEAD 200 responses accumulate matches.
- Changing responses accumulate mismatches.
- Mismatches are explainable in dashboard and events.

## Auto Mode Flow

```text
client request
  -> request safety precheck
  -> panic switch check
  -> cache lookup when eligible
  -> if fresh verified entry exists: serve reuse
  -> otherwise origin request
  -> response safety extraction
  -> fingerprint if bounded and eligible
  -> observe/shadow sample
  -> store if allowed
  -> client response from origin
```

Rules:

- Run request safety checks before every cache lookup.
- Only GET/HEAD requests can reach cache lookup.
- A cached entry can be served only when:
  - route/key is verified for auto reuse
  - entry is fresh
  - entry fingerprint is available
  - panic switch is inactive
  - request still has no hard-deny signals
  - Vary-selected request header values match the key
- Expired or missing entries cause origin pass-through.
- Store failures never fail the response.

Auto mode acceptance:

- Protected requests always go to origin.
- Unknown routes go to origin.
- Verified public stable responses can be reused within freshness TTL.
- Debug headers are emitted only when enabled.

## Origin Request Flow

Origin forwarding should:

- Preserve method, path, query, and body.
- Rewrite scheme/authority to the configured origin.
- Preserve relevant client headers.
- Set or append `Via` if desired, but do not require it for v0.1.0.
- Stream request body to origin.
- Apply origin timeout from config.
- Return `502 Bad Gateway` for connection errors.
- Return `504 Gateway Timeout` for timeout.

Origin errors should be observable as bypass/origin-failure events without leaking sensitive request data.

## Response Handling

For origin responses:

1. Capture status and headers.
2. Extract response safety signals:
   - status cacheability
   - has `Set-Cookie`
   - cache-control class
   - vary class
   - content length
   - content type
3. Decide whether the body may be buffered for fingerprint/cache.
4. Stream directly when not eligible or too large.
5. Compute fingerprint for eligible bounded responses.
6. Store cache entry only after policy allows storage.
7. Return response to client.

## Buffering and Streaming Rules

Default limits:

```yaml
storage:
  max_object_size: 1MiB
policy:
  max_fingerprint_body_size: 2MiB
```

Rules:

- Unsafe methods stream without cache buffering.
- Responses with `Set-Cookie`, `private`, `no-store`, `no-cache`, or `Vary: *` stream without storage.
- Responses above `max_object_size` are not stored.
- Responses above `max_fingerprint_body_size` are not promoted to auto reuse.
- If content length is known and too large, skip buffering.
- If content length is unknown, buffer only until limit; on limit overflow, discard buffer and stream/pass through.

Implementation note: for v0.1.0 it is acceptable to buffer small eligible responses fully before returning, as long as large/ineligible responses stream and performance targets still pass.

## Cache Lookup and Response Construction

Cache lookup input is a hash of the full cache key. The raw cache key may be held in memory only as long as needed to index the store and must not be exposed in metrics.

On cache hit:

- Validate `expires_at > now`.
- Clone status, headers, and body bytes.
- Remove hop-by-hop response headers before client response.
- Optionally add `X-Kubio-Status: hit` when debug headers are enabled.
- Record route reuse count and latency.

On stale hit:

- Treat as miss.
- Evict opportunistically or mark expired.
- Fetch from origin.

## Cache Store Flow

Store only when policy returns `StoreOnly` or when auto-mode origin response should refresh a verified entry.

Cache entry fields:

```text
status
headers
body bytes
created_at
expires_at
fingerprint
route_id
cache_key_hash
```

Before storage:

- Remove hop-by-hop headers.
- Avoid storing `Set-Cookie`.
- Exclude debug headers.
- Respect max object size.
- Set TTL from active freshness profile.

## Debug Headers

Disabled by default. When `--debug-headers` is enabled:

```http
X-Kubio-Status: hit
X-Kubio-Status: miss
X-Kubio-Status: protected
X-Kubio-Status: bypass
```

Do not include raw reasons or identifiers in response headers for v0.1.0.

## Panic Switch

If configured and active:

- Never serve cached responses.
- Do not promote routes to auto.
- Continue forwarding to origin.
- Observation may continue if it remains privacy-safe.
- Emit `panic_switch_enabled` once per active transition.

## Timing and Metrics Points

Record:

- Total request duration.
- Origin duration for origin-bound requests.
- Decision counters.
- Reused/protected/bypass/origin counters.
- Store get/put errors.
- Response status class.

Metrics recording must not block response completion.

## Edge Cases

- `HEAD`: can be eligible, but body storage semantics differ. v0.1.0 should store metadata only for HEAD or reuse a GET-derived entry only after explicit implementation and tests. Conservative default: HEAD can be observed and protected, but auto reuse requires dedicated tests.
- Range requests: protect or bypass in v0.1.0.
- Request bodies on GET: forward to origin and do not reuse.
- `Cache-Control: no-cache`: do not auto reuse in v0.1.0.
- `Vary` values beyond a small allowlist: bypass reuse unless safely keyed.
- Compressed responses: include `Accept-Encoding` via `Vary` handling or avoid reuse when ambiguous.
