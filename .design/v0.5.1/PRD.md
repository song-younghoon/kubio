# PRD: kubio v0.5.1

Document status: implemented
Target release: `v0.5.1`
Core philosophy: **increase hit rate by proving finer equivalence**

Implementation state: goals and safety constraints are implemented on `main`;
local workspace and targeted precision regression gates are expected release
gates.

## 1. Product Summary

kubio v0.5.1 should refine the v0.5.0 adaptive reuse model so safe public API
traffic reaches useful cache-hit rates in more real-world shapes:

- public object routes with noisy query parameters;
- content routes that use slugs instead of numeric IDs;
- routes whose safety evidence changes over time;
- routes with bounded variants such as configured language or encoding keys.

The user-facing difference is better explanations and fewer missed hits on
public data. v0.5.1 should not make private, authenticated, cookie-bearing, or
unsafe traffic cacheable by default.

## 2. Background

v0.5.0 solved the first major reuse bottleneck: a route such as `/notice/{id}`
can become a public object route, while `/user/{id}` remains protected.

After that change, the next hit-rate losses are more subtle:

- `?utm_source=...`, `?gclid=...`, and other tracking parameters create
  separate cache keys even when responses match;
- public content routes often use slugs, for example `/news/summer-release`,
  which may not be normalized as object-shaped;
- a route can be promoted from old evidence even after traffic changes unless
  confidence ages or is rechecked;
- unsupported or unconfigured variants make the explanation too coarse;
- operators need to know whether they should add a route hint, ignore a query
  parameter, wait for more evidence, or treat the route as genuinely unsafe.

v0.5.1 should turn those cases into deterministic evidence and bounded
operator-facing controls.

## 3. Goals

v0.5.1 should:

1. Add a precision confidence model that separates store, serve, and key-shape
   eligibility.
2. Keep route confidence fresh with bounded windows, decay, and cooldown.
3. Prove query parameter equivalence with fingerprint evidence before any key
   compaction.
4. Keep query key compaction disabled by default unless explicitly enabled or
   applied through a route hint.
5. Block automatic query compaction for sensitive parameter names.
6. Add slug-like public object route detection with conservative thresholds.
7. Make variant dimensions explicit and bounded in snapshots and explanations.
8. Add sampled canary validation for promoted route and query-equivalence
   classes.
9. Add route/equivalence-group demotion and purge behavior for negative
   evidence.
10. Add benchmarks that show v0.5.1 improves over v0.5.0 on query-noisy and
    slug public routes while preserving protected-user behavior.

## 4. User Experience

### 4.1 Query-Noisy Public Object Route

An origin exposes public notices:

```text
GET /notice/1?utm_source=a
GET /notice/1?utm_source=b
GET /notice/1?utm_source=c
```

Expected behavior:

- kubio records query parameter presence and bounded value hashes.
- If fingerprints match across enough query variants, kubio marks `utm_source`
  as a `verified_ignore_candidate`.
- By default, kubio explains the opportunity but does not collapse keys.
- If the operator enables v0.5.1 query key compaction for that route, future
  requests can share a cache entry that ignores `utm_source`.
- A later mismatch demotes the equivalence group and purges affected entries.

### 4.2 Sensitive Query Parameter

```text
GET /notice/1?token=abc
```

Expected behavior:

- `token` is never automatically ignored.
- The dashboard reports `sensitive_query_param`.
- Any explicit route hint must still pass hard request/response safety checks.

### 4.3 Slug Public Object Route

```text
GET /articles/summer-release
GET /articles/winter-update
```

Expected behavior:

- kubio can classify the final segment as slug-like only after bounded evidence.
- The route may become `public_object_candidate` when static resource names are
  public-looking, responses are store-safe, and fingerprints are stable.
- Sensitive resources such as `/users/jane-doe` remain hard protected.

### 4.4 Evidence Decay

Expected behavior:

- A route promoted last week but idle since then may degrade from
  `public_object_strong` to `public_object_probation` or `watching`.
- The next safe origin samples can restore confidence.
- Decay never causes kubio to serve unsafe data; it only makes kubio more
  likely to pass through to origin until evidence is fresh again.

### 4.5 Canary Validation

Expected behavior:

- A small configurable sample of promoted-route requests goes to origin for
  verification instead of serving a hit.
- Matching fingerprints refresh confidence.
- A mismatch demotes and purges deterministically.

## 5. Non-Goals

v0.5.1 will not:

- cache authenticated responses;
- cache cookie-bearing requests by default;
- implement per-user cache partitions;
- automatically cache GraphQL responses;
- use machine learning or remote classification;
- share evidence across processes;
- ignore arbitrary query parameters without proof;
- treat slugs as dynamic on sensitive resource routes.

## 6. Product Principles

### 6.1 Precision Before Expansion

Each new hit-rate improvement must identify which equivalence it proved:
same exact key, same route, same query-equivalence group, or same bounded
variant.

### 6.2 Key Compaction Is More Sensitive Than Route Promotion

Route promotion still stores distinct raw keys. Query key compaction changes
which requests share an entry, so it requires stricter proof, explicit config,
and deterministic rollback.

### 6.3 Evidence Must Age

Traffic changes. Positive evidence should expire or decay, while negative
evidence should trigger immediate demotion and a cooldown.

### 6.4 Operators Need Next Actions

Explanations should say whether the route needs more samples, a route hint,
query key compaction enablement, variant configuration, or no action because it
is hard protected.

## 7. Success Metrics

The release is successful when:

- Query-noisy public object benchmarks show materially higher hit rate than
  v0.5.0 after explicit key-shaping enablement.
- Slug public object benchmarks can promote safe routes without promoting
  sensitive slug routes.
- Evidence decay tests show stale confidence degrades to origin pass-through.
- Canary mismatch tests demote and purge affected entries.
- Sensitive query parameters never become auto-ignore candidates.
- `/user/{id}` and `/users/{slug}` remain protected by default.
- Existing v0.5.0 exact-key, public-object, origin-public, revalidation, stale,
  protocol, and storage tests remain green.
