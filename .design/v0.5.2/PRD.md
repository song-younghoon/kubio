# PRD: kubio v0.5.2

Document status: implemented
Target release: `v0.5.2`
Core philosophy: **increase hit rate by separating response metadata from
representation identity**

## 1. Product Summary

kubio v0.5.2 improves practical cache hit rate for stable public API
responses whose response headers include per-request metadata. Common examples
are `x-response-id`, `x-correlation-id`, cloud trace IDs, and request tracing
headers.

The user-facing difference is that routes such as `GET /notice/1` can reuse
when only one-shot response metadata changes. Routes such as `GET /user/1`
remain protected by the same hard request and path rules.

## 2. Background

v0.5.1 fingerprints a response using status, a stable header hash, and body
hash. That is safer than body-only validation because headers can carry
representation and cache semantics.

In real API traffic, however, response headers often include values that do not
describe the representation:

- one-shot response IDs;
- request or correlation IDs echoed by the origin;
- distributed tracing headers;
- cloud/load-balancer request identifiers;
- gateway metadata such as `Via`, `Server`, or changing `Date`.

Some of these are already excluded, but the model is implicit and incomplete.
`x-response-id` is the motivating gap. The fix should be explicit enough that
operators can understand why a header was ignored, suggested, or treated as
fingerprint-sensitive.

## 3. Goals

v0.5.2 should:

1. Add a documented response-header taxonomy.
2. Expand the default volatile metadata header set for common request,
   response, correlation, and trace IDs.
3. Keep cache-safety, validators, and representation headers
   fingerprint-sensitive by default.
4. Add route/header evidence for verified volatile candidates outside the
   default set.
5. Keep automatic unknown-header ignoring disabled by default.
6. Add route hints for enabling verified header ignore candidates.
7. Strip one-shot volatile headers from cache-hit responses unless explicitly
   preserved.
8. Add observability that explains header normalization without exposing values.
9. Add benchmarks proving improved hits for stable public endpoints with
   dynamic response metadata.

## 4. User Experience

### 4.1 Stable Public Response With Dynamic IDs

Origin behavior:

```text
GET /notice/1

200 OK
Content-Type: application/json
X-Response-Id: res-a

{"id":1,"title":"maintenance"}
```

Next origin sample:

```text
200 OK
Content-Type: application/json
X-Response-Id: res-b

{"id":1,"title":"maintenance"}
```

Expected behavior:

- `x-response-id` is classified as volatile response metadata.
- The response fingerprint ignores `x-response-id`.
- Status, body, and representation/cache headers still have to match.
- The route/key can pass existing v0.5.1 reuse gates.
- A cache hit does not replay `res-a`; by default the header is omitted from
  hit responses.

### 4.2 Dynamic Date Header

`Date` remains excluded from fingerprint comparison. Cache hits may preserve
normal cache semantics by returning the stored `Date` with an `Age` header, or
by applying the v0.5.2 configured hit-time behavior.

Expected behavior:

- changing `Date` alone does not cause shadow/canary mismatch;
- hit responses do not invent origin request IDs;
- freshness still follows kubio's configured freshness and revalidation policy.

### 4.3 Unknown Vendor Header

Origin behavior:

```text
X-Vendor-Execution-Id: a
X-Vendor-Execution-Id: b
```

If status, body, and all sensitive headers remain stable, kubio may classify
this as a `verified_volatile_candidate`.

Default behavior:

- kubio reports the opportunity;
- the header is not ignored automatically unless it matches the curated default
  volatile set;
- the operator can enable it for a route.

Example route hint:

```yaml
routes:
  - match:
      method: GET
      path: /notice/{id}
    response_headers:
      verified_ignore:
        enabled: true
        allow: ["x-vendor-execution-id"]
```

### 4.4 Semantically Important Header Changes

If any of these change, kubio must keep treating the response as different or
unsafe:

```text
Cache-Control
Vary
Content-Type
Content-Encoding
ETag
Last-Modified
Set-Cookie
Location
```

Expected behavior:

- `Set-Cookie`, `private`, `no-store`, unsupported `Vary`, and `Vary: *` still
  block storage and reuse.
- representation header changes cause mismatch or demotion.
- validator equivalence is not part of v0.5.2.

### 4.5 Protected Routes Stay Protected

```text
GET /user/1
X-Response-Id: res-a
```

Expected behavior:

- the sensitive path remains protected;
- `Authorization` and `Cookie` still hard-protect requests;
- response-header normalization never overrides request safety.

## 5. Non-Goals

v0.5.2 will not:

- cache authenticated responses;
- cache cookie-bearing requests by default;
- implement per-user cache partitions;
- infer semantics for arbitrary business headers;
- ignore cache-control or representation headers by default;
- make header values visible in metrics, events, snapshots, CLI, or debug
  output;
- add distributed evidence sharing.

## 6. Product Principles

### 6.1 Metadata Is Not Representation

Per-request IDs and trace headers should not block reuse when status, body, and
semantic headers are stable.

### 6.2 Unknown Headers Require Proof and Enablement

Unknown headers can carry business state. kubio can suggest candidates after
evidence, but default automatic ignoring stays limited to a curated metadata
set.

### 6.3 Fingerprint Ignoring and Hit Headers Are Different

Excluding a header from the fingerprint does not mean replaying it from cache is
correct. One-shot identifiers should be stripped on cache hits by default.

### 6.4 Safety Headers Stay Above Heuristics

`Set-Cookie`, `Cache-Control`, `Vary`, validators, representation headers, and
hard request denies remain stronger than header-equivalence evidence.

## 7. Success Metrics

The release is successful when:

- a benchmark with stable bodies and changing `x-response-id` shows materially
  higher hit rate than v0.5.1;
- existing v0.5.1 adaptive reuse, query equivalence, slug, canary,
  revalidation, stale-if-error, protocol, and storage tests remain green;
- protected user routes still produce zero hits and zero stores by default;
- `Set-Cookie`, `private`, `no-store`, `Vary: *`, unsupported `Vary`, and
  representation header changes still block or demote reuse;
- cache hits do not replay stripped one-shot response identifiers;
- snapshots, metrics, events, debug headers, disk metadata, and CLI output do
  not contain raw volatile header values.
