# kubio v0.5.0 Design Index

Status: implemented
Source: v0.4.1 runtime baseline and reuse-rate feedback
Target release: `v0.5.0`

Implementation state: adaptive reuse, path intelligence, observability, docs,
examples, benchmarks, and release hardening gates are implemented on `main`.

v0.5.0 is a proxy behavior release. v0.4.x made kubio easier to install and
update; v0.5.0 should make automatic reuse effective enough to matter in real
traffic without weakening the hard safety rules that protect personalized or
mutable responses.

The release theme is:

```text
Evidence-based reuse: keep hard denies hard, but stop requiring every safe
public object key to prove itself from zero.
```

## Problem Statement

The current auto path is safe but too strict for common object endpoints. A
route such as:

```text
GET /notice/1
GET /notice/2
GET /notice/3
```

is normalized into one route template for observation, but each raw path remains
a separate cache key. The route and each key must pass conservative shadow
thresholds before kubio stores and serves hits. With default thresholds, many
public object endpoints never accumulate enough exact-key repeats to reuse.

The correct v0.5.0 target is not to reuse risky traffic. It is to recognize
routes that behave like public object collections and let route-level evidence
help new keys. `/notice/{id}` should become useful after enough safe evidence.
`/user/{id}` should remain protected by default.

## Design Documents

- [PRD](PRD.md)
  - Product goals, user experience, non-goals, and success metrics.
- [Adaptive Reuse Policy](adaptive-reuse-policy.md)
  - Hard-deny taxonomy, route/key evidence, public object promotion, fast paths,
    demotion, purge behavior, and config.
- [Path Intelligence](path-intelligence.md)
  - Path segment cardinality, sensitive resource classification, public object
    candidate detection, privacy constraints, and examples.
- [Observability and Dashboard](observability-dashboard.md)
  - Dashboard/API fields, route explanations, metrics, events, CLI output, and
    debug headers for the new reuse model.
- [Testing and Release](testing-release.md)
  - Unit, integration, benchmark, safety, privacy, and release gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone work breakdown with acceptance checks.

## In Scope

- Split request/response policy into hard denies and evidence-gated soft risks.
- Add route-level public object evidence so dynamic public endpoints can reuse
  after route confidence is established.
- Add key-level validation so repeated exact keys can reuse without waiting for
  full route auto promotion.
- Add origin-public fast path for explicitly public cacheable origin responses.
- Add path intelligence for dynamic path cardinality and sensitive resource
  classification.
- Keep cache keys path-specific. Route-level evidence can open eligibility, but
  `/notice/1` and `/notice/2` must remain different cache entries.
- Add explanations for blocked reuse that distinguish hard protection from
  insufficient route evidence, insufficient key evidence, and unstoreable origin
  responses.
- Add benchmark scenarios that prove hit-rate improvements on public object
  routes.

## Out of Scope

- Reuse for requests with `Authorization`.
- Default reuse for requests with `Cookie`.
- Reuse for unsafe methods or GraphQL mutations.
- Serving cache entries across different raw paths.
- Reusing responses with `Set-Cookie`, `Cache-Control: no-store`, `private`,
  `Vary: *`, unsupported `Vary`, missing fingerprints, or shadow mismatches.
- Distributed cache coordination, Redis, or multi-process route evidence.
- User-specific authenticated caches.
- Machine learning classification.

## Cross-Cutting Constraints

- Hard denies remain hard by default: Authorization, Cookie, unsafe methods,
  Range, GET/HEAD bodies, Set-Cookie, no-store, private, unsupported Vary,
  Vary wildcard, status not cacheable, fingerprint unavailable, oversized
  objects, panic switch, and shadow mismatches.
- Sensitive resource names remain protected by default. `user`, `users`,
  `account`, `profile`, `session`, `login`, `admin`, `billing`, `payment`,
  `token`, and `oauth` must not become public object routes without an explicit
  route hint and the existing hard-deny checks.
- Route-level evidence may allow storing and reusing keys sooner, but it must
  never serve a response for a key before that key has been fetched and stored.
- Shadow mismatch demotion must be deterministic. A mismatch on a route promoted
  through evidence should demote the route and purge affected stored entries.
- Observability must not expose raw dynamic path segment values, query values,
  authorization values, cookie values, validator values, or response bodies.
- Metrics labels remain bounded to method, route template, decision, outcome,
  reason class, status class, store kind, and histogram buckets.
- Fail open remains the default for internal uncertainty: pass through to
  origin instead of serving stale or unverified cache data.

## Milestone Status

- [x] M0: Design, terminology, and schema lock.
- [x] M1: Policy taxonomy and route/key eligibility model.
- [x] M2: Path intelligence and public object classification.
- [x] M3: Store/reuse flow changes and demotion/purge behavior.
- [x] M4: Demotion and purge.
- [x] M5: Dashboard, metrics, CLI, docs, and examples.
- [x] M6: Benchmarks, safety tests, and release hardening.
