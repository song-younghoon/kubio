# kubio v0.5.2 Design Index

Status: implemented
Source: v0.5.1 precision adaptive reuse implementation
Target release: `v0.5.2`

v0.5.0 made public object routes useful. v0.5.1 made that reuse more precise
with query equivalence, slug evidence, evidence decay, and canary validation.
Implementation state: response-header equivalence config, policy-aware
fingerprints, default volatile metadata ignores, route-enabled response header
ignores, hit-time volatile stripping, observability, benchmarks, docs, and
release notes are implemented on `main`.

v0.5.2 removes another practical hit-rate blocker: response metadata
headers that change on every origin response even when the representation is
the same.

The release theme is:

```text
Header-normalized reuse: ignore proven response metadata, not representation
semantics.
```

## Problem Statement

Modern API servers commonly add response metadata such as:

```text
Date: Sun, 31 May 2026 10:00:00 GMT
X-Request-Id: req-...
X-Response-Id: res-...
X-Correlation-Id: ...
Traceparent: ...
```

Those values often change per request. If kubio treats them as part of the
response fingerprint, otherwise stable public endpoints look unstable and stay
in observe/store-only paths. A route such as `GET /notice/1` can miss even when
the status, body, cache semantics, and representation headers are stable.

The current runtime already excludes some volatile names such as `date` and
`x-request-id`, but the list is narrow and cache hits may replay one-shot
origin identifiers if those headers were stored. v0.5.2 should expand this into
an explicit, configurable response-header equivalence model.

The goal is not to ignore arbitrary headers. The goal is to distinguish:

- headers that describe the representation and must remain fingerprinted;
- headers that control cache safety and must remain hard policy signals;
- headers that are request/trace metadata and can be excluded safely;
- unknown headers that may be suggested only after evidence.

## Design Documents

- [PRD](PRD.md)
  - Product goals, user experience, non-goals, and success metrics.
- [Response Header Equivalence](response-header-equivalence.md)
  - Header taxonomy, fingerprint normalization, evidence rules, config, and
    demotion behavior.
- [Header Sanitization and Store Contract](header-sanitization-and-store.md)
  - How stored headers differ from fingerprint headers, hit-time stripping,
    metadata versioning, and compatibility.
- [Observability and Dashboard](observability-dashboard.md)
  - Snapshot fields, CLI output, debug headers, events, and metrics.
- [Testing and Release](testing-release.md)
  - Unit, integration, benchmark, privacy, compatibility, and release gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone work breakdown with acceptance checks.

## In Scope

- Add an explicit response-header fingerprint policy.
- Expand the default volatile metadata list for common request, response,
  correlation, and trace identifiers.
- Keep cache-safety and representation headers fingerprint-sensitive by
  default.
- Add evidence for unknown or route-specific volatile header candidates.
- Add route-level enablement for verified ignored response headers.
- Strip or rewrite one-shot metadata headers on cache hits so cached responses
  do not replay stale request IDs.
- Add dashboard, CLI, debug header, metrics, and event explanations for header
  normalization decisions.
- Add benchmarks for public endpoints with dynamic response metadata headers.

## Out of Scope

- Authenticated or per-user response caching.
- Default reuse for cookie-bearing requests.
- Ignoring `Set-Cookie`, `Cache-Control`, `Vary`, unsupported variants, or
  private/no-store responses.
- Ignoring arbitrary `x-*` headers without evidence.
- Serving user-specific trace, request, or response identifiers from cache.
- Full RFC cache compliance beyond kubio's existing freshness and revalidation
  model.
- Distributed header-equivalence evidence.

## Cross-Cutting Constraints

- v0.5.1 hard denies remain hard: Authorization, Cookie, unsafe methods, Range,
  GET/HEAD bodies, Set-Cookie, no-store, private, unsupported `Vary`,
  `Vary: *`, uncacheable status, missing fingerprint, oversized object, panic
  switch, shadow mismatch, and canary mismatch.
- Header normalization must never make a response store-safe when existing
  response policy says it is not store-safe.
- Status and body hash remain part of every response fingerprint.
- Representation headers such as `Content-Type`, `Content-Encoding`,
  `Content-Language`, `Content-Range`, and `Location` remain fingerprinted by
  default.
- Cache policy and validator headers such as `Cache-Control`, `Vary`, `ETag`,
  `Last-Modified`, and `Expires` remain fingerprinted by default unless a later
  release explicitly designs validator-specific equivalence.
- Unknown headers may become suggestions after evidence, but automatic default
  ignoring is limited to a curated non-semantic metadata list.
- Cache hits must not replay stripped one-shot identifiers such as
  `x-response-id`.
- Observability must expose header names, classes, counts, and bounded hashes,
  not header values.

## Milestone Status

- [x] M0: Design, terminology, and schema lock.
- [x] M1: Header taxonomy and config.
- [x] M2: Fingerprint normalization.
- [x] M3: Header equivalence evidence and demotion.
- [x] M4: Store and hit-time header sanitization.
- [x] M5: Dashboard, metrics, CLI, docs, and examples.
- [x] M6: Benchmarks, safety tests, compatibility tests, and release hardening.
