# PRD: kubio v0.2.0

Document version: v0.2 implemented baseline
Product type: Open-source software
Primary implementation language: Rust
Release target: Safer real-world API response reuse
Core philosophy: **preserve safety, use origin validators, expose operator intent, persist locally**

---

## 1. Product Summary

kubio v0.2.0 extends the v0.1.0 local-first API response reuse autopilot with the features needed to keep safe responses useful after their first freshness window:

```text
conditional revalidation
bounded stale-if-error
route-level operator hints
query parameter intelligence
optional disk persistence
```

The core promise becomes:

> kubio can keep reusing public stable API responses across freshness windows, restarts, and brief origin failures, while continuing to protect risky traffic by default.

## 2. Background and Problem

v0.1.0 intentionally takes a narrow approach:

- Fresh memory entries can be reused only after conservative shadow validation.
- Stale entries go to origin.
- `Cache-Control: no-cache` is not reused.
- All cache state is lost on restart.
- Query parameters are always preserved in the cache key.

Those defaults are safe, but they limit impact in common API deployments:

- Many origins already publish `ETag` or `Last-Modified` validators.
- Many responses can be stored but must be revalidated before reuse.
- Operators often know that specific routes can tolerate stale data during outages.
- Tracking query parameters can fragment otherwise reusable public responses.
- Local development and single-node deployments benefit from a disk-backed cache.

v0.2.0 adds these capabilities without changing kubio's safety posture.

## 3. Product Goals

kubio v0.2.0 should:

```text
1. Revalidate stale cached responses using ETag and Last-Modified.
2. Treat Cache-Control: no-cache as storeable only when revalidation happens before reuse.
3. Refresh cached metadata on 304 Not Modified.
4. Replace cached entries on 200 OK revalidation responses.
5. Serve stale responses during origin failure only with explicit permission and bounded age.
6. Let operators add route-level hints for freshness, query keys, Vary allowlists, and stale recovery.
7. Show query parameter observations and conservative safe-ignore suggestions without automatically hiding risk.
8. Provide a process-local disk store that survives restart.
9. Preserve v0.1.0 hard-deny behavior for personalized and unsafe traffic.
10. Explain every new decision through dashboard, CLI, JSON APIs, events, and metrics.
```

## 4. User Experience Goals

### Revalidation

When an eligible cached response becomes stale, kubio should revalidate instead of immediately treating it as a miss:

```text
GET /api/products

Cached response is stale.
kubio revalidated with ETag.
Origin returned 304 Not Modified.
kubio reused the stored body and refreshed freshness metadata.
```

### Stale-if-error

When allowed by origin headers or route policy, kubio should make outage behavior visible:

```text
GET /api/catalog

Origin timed out.
kubio served a verified stale response because stale-if-error is allowed for this route.
Stale age: 42s
Maximum allowed stale age: 5m
```

When stale serving is not allowed:

```text
GET /api/catalog

Origin timed out.
kubio did not serve stale because no stale-if-error permission was configured.
Client received 504 Gateway Timeout.
```

### Route Hints

Operators should be able to express intent in YAML:

```yaml
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

Hints must read as intent, not as permission to bypass core safety.

## 5. Non-Goals

kubio v0.2.0 will not provide:

```text
Redis-compatible shared cache
Multi-node cache coherence
Kubernetes operator
GraphQL POST reuse
Authenticated per-user cache
Unsafe method reuse
Hosted dashboard
Config reload without restart
Encrypted disk cache
Full RFC-complete HTTP cache semantics
Automatic query parameter ignoring without validation or explicit configuration
```

## 6. Product Principles

### 6.1 Validators Before Stale Assumptions

If a cached entry is stale and has validators, kubio should ask the origin before reuse.

Allowed validators:

- `ETag`
- `Last-Modified`

Unsupported or malformed validators cause origin pass-through.

### 6.2 Stale Recovery Is Explicit

Fresh reuse can remain automatic after shadow validation. Stale reuse during errors requires one of:

- Origin response permits it with `Cache-Control: stale-if-error=<seconds>`.
- Route hint enables it with a bounded `max_stale`.

No implicit stale serving.

### 6.3 Hints Cannot Relax Hard Denies

Normal route hints may tune safe behavior, but cannot permit reuse for:

- Requests with `Authorization`.
- Requests with `Cookie`.
- Unsafe methods.
- Responses with `Set-Cookie`.
- Responses with `Cache-Control: no-store`.
- Responses with `Cache-Control: private`.
- `Vary: *` or unsupported `Vary`.
- Range requests.
- Shadow mismatches.

Sensitive-looking paths may be acknowledged by a route hint only when the route is still public by request/response signals.

### 6.4 Persistence Is Still Local

Disk storage extends process-local cache lifetime across restarts. It does not introduce distributed consistency, cross-node invalidation, or shared locks.

### 6.5 Corruption Never Expands Reuse

Unsafe revalidation metadata and corrupt disk metadata must narrow reuse rather than broaden it. If a 304 response introduces hard-deny metadata, kubio purges the stored entry and refetches. If disk metadata references a body file that does not match its cache key, kubio skips and deletes that entry instead of reading arbitrary files.

## 7. Success Metrics

Release success is measured by:

- Revalidation tests prove 304 responses reuse stored bodies and update metadata.
- `no-cache` responses are never served without revalidation.
- Stale-if-error tests prove stale is served only when allowed and bounded.
- Disk store restart tests prove safe entries survive restart and protected entries do not persist.
- Disk corruption tests prove corrupt metadata cannot trigger unsafe reuse or arbitrary body file reads.
- Query hint tests prove ignored parameters affect keys only when explicitly configured.
- Existing v0.1.0 safety integration tests continue to pass.

## 8. Compatibility

Default behavior should remain close to v0.1.0:

- Default mode is still `watch`.
- Memory store remains the default.
- Hard denies remain unchanged.
- Query parameters remain included unless configured otherwise.
- Stale serving is not implicit.

v0.2.0 config should reject unknown unsafe shortcuts rather than silently accepting them.
