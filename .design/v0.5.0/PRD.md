# PRD: kubio v0.5.0

Document status: implemented
Target release: `v0.5.0`
Core philosophy: **raise cache hit rate through evidence, not optimism**

Implementation state: goals and safety constraints are implemented on `main`;
local workspace, HTTP/3 feature, integration, and adaptive benchmark gates have
passed.

## 1. Product Summary

kubio v0.5.0 should make automatic response reuse effective on common public API
endpoints, especially dynamic object routes such as `/notice/1`,
`/articles/42`, and `/products/123`.

The release should preserve kubio's conservative trust model. The user-facing
change is that safe public routes should start producing hits after a small,
bounded amount of evidence instead of requiring every individual object key to
accumulate enough repeated traffic on its own.

## 2. Background

Current kubio behavior protects risky requests and responses early, validates
response fingerprints in shadow, and reuses only after route and key thresholds
are satisfied. This is safe, but in real traffic it frequently means:

- public object endpoints remain stuck in observation;
- high-cardinality ID routes have almost no exact-key repeats;
- operators see protected or low-benefit explanations even when the endpoint is
  clearly a public read endpoint;
- the actual reuse rate approaches zero unless traffic is extremely repetitive
  or the user manually writes route hints.

v0.5.0 should close that gap with deterministic evidence. Cardinality is useful
because it identifies object-shaped routes. It is not sufficient by itself. The
route must also show public, store-safe, stable behavior.

## 3. Goals

v0.5.0 should:

1. Increase cache hit rate for safe public GET/HEAD endpoints.
2. Allow route-level evidence to help high-cardinality public object routes.
3. Keep `/user/{id}` and similar sensitive resource routes protected by default.
4. Preserve hard denies for Authorization, Cookie, unsafe methods, unsafe
   response headers, unsupported Vary, and shadow mismatches.
5. Add key-level reuse eligibility for exact keys with repeated matching
   fingerprints.
6. Add an origin-public fast path when the origin explicitly marks a response
   public and cacheable.
7. Add path cardinality and route classification to observer snapshots.
8. Explain why a route is not reusing in terms users can act on.
9. Add repeatable benchmarks that demonstrate hit-rate improvements compared to
   v0.4.1.
10. Keep all new observation metadata bounded and privacy-preserving.

## 4. User Experience

### 4.1 Public Object Route

An origin exposes public notices:

```text
GET /notice/1 -> 200 OK, JSON, no Set-Cookie, no private/no-store
GET /notice/2 -> 200 OK, JSON, no Set-Cookie, no private/no-store
```

Expected v0.5.0 behavior in auto mode:

- kubio observes early requests and records route/key evidence.
- Once the route has enough store-safe samples and zero mismatches, it becomes a
  public object route.
- New notice IDs are stored after their first safe origin response.
- Repeated notice IDs can hit on the next fresh request.
- The dashboard explains the route as `public_object`.

### 4.2 Sensitive Object Route

An origin exposes user profiles:

```text
GET /user/1
```

Expected behavior:

- kubio protects the route by default because `user` is a sensitive resource
  segment.
- Cardinality does not override sensitive resource classification.
- A route hint may acknowledge the sensitive-looking path only when the operator
  owns the risk, but Authorization, Cookie, Set-Cookie, private, no-store,
  unsupported Vary, and shadow mismatch still cannot be bypassed.

### 4.3 Origin Public Fast Path

An origin returns:

```text
Cache-Control: public, max-age=60
```

Expected behavior:

- kubio may store the first safe response even before route auto promotion.
- The second fresh request for the same key may hit.
- TTL remains bounded by kubio's freshness policy and object-size limits.

### 4.4 Explanations

Dashboard and CLI should distinguish:

```text
hard_protected: has_cookie
hard_protected: sensitive_resource
waiting_for_route_evidence
waiting_for_key_evidence
origin_not_store_safe
public_object_candidate
public_object
origin_public
demoted_shadow_mismatch
```

## 5. Non-Goals

v0.5.0 will not:

- Reuse authenticated responses.
- Reuse cookie-bearing requests by default.
- Implement per-user caches.
- Reuse POST/PUT/PATCH/DELETE responses.
- Collapse distinct raw paths into one cache entry.
- Ignore query parameters automatically by default.
- Introduce a hosted control plane or required telemetry.
- Implement a distributed route-evidence store.

## 6. Product Principles

### 6.1 Evidence Before Reuse

kubio may become less strict about how much evidence each individual key needs,
but it should not serve unobserved or unsafe data. Every served cache entry must
come from a prior safe origin response for the same cache key.

### 6.2 Keep Hard Denies Simple

Hard denies should remain easy to explain and audit. They should not depend on
probabilistic scoring.

### 6.3 Route Evidence Helps Object Keys

High-cardinality routes should be evaluated at the route-template level. Once a
route proves it behaves like public immutable or slowly changing objects, new
keys should not start from the same conservative baseline as an unknown route.

### 6.4 Sensitive Names Beat Cardinality

High cardinality is common for public content and private user data. Sensitive
resource names remain protected by default even if cardinality is high and
fingerprints appear stable.

### 6.5 Operators Need Actionable Explanations

The dashboard should tell users which threshold is blocking reuse and what kind
of route kubio believes it is observing.

## 7. Success Metrics

The release is successful when:

- A local benchmark for `/notice/{id}` shows materially higher hit rate than
  v0.4.1 after route evidence is established.
- A repeated exact-key benchmark hits by the third request under default v0.5.0
  settings when all hard safety checks pass.
- `/user/{id}` remains protected by default in integration tests.
- Authorization and Cookie tests still show zero stored entries and zero reused
  responses.
- Shadow mismatch demotes public object routes and prevents further hits.
- The dashboard/API can explain route class, path cardinality, evidence counts,
  and blocking reasons without raw path IDs.
- Existing v0.4.1 protocol, store, revalidation, stale-if-error, install, and
  update tests remain green.
