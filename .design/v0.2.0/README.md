# kubio v0.2.0 Design Index

Status: implemented baseline and safety-hardened
Source: v0.1.0 implementation baseline and `docs/roadmap.md`
Target release: `v0.2.0`

This directory defines the v0.2.0 design for kubio. v0.1.0 proved the local-first reverse proxy, conservative policy engine, shadow validation, memory store, dashboard, and metrics path. v0.2.0 should keep those safety defaults while making reused responses more useful in real API deployments.

The release theme is:

```text
Safer real-world reuse through revalidation, bounded stale recovery, operator hints, and local persistence.
```

## Release Definition

kubio v0.2.0 is complete when a user can:

- Revalidate stale eligible responses with `ETag` and `Last-Modified`.
- Safely store `Cache-Control: no-cache` responses only when they can be revalidated before reuse.
- Serve stale verified public responses during origin failure only when origin headers or route policy explicitly allow it.
- Configure route-level policy hints for freshness, query parameters, and stale recovery without bypassing hard safety denies.
- See query parameter names, configured actions, and conservative noise-parameter suggestions in dashboard/API output.
- Choose `storage.kind: disk` for process-local persistent cache entries.
- Restart kubio and keep safe disk-backed entries without persisting sensitive observations.
- Understand every revalidation, stale, query, and disk-store decision from CLI/dashboard output and metrics.

Post-baseline hardening added:

- Unsafe 304 metadata purges stale entries before refetch.
- Disk metadata cannot point to arbitrary body paths.
- Route hint validation rejects duplicate routes, empty query blocks, and overlapping include/ignore globs.
- Query observation can be disabled with `policy.query_intelligence.enabled: false`.

## In Scope

- Conditional revalidation with `If-None-Match` and `If-Modified-Since`.
- Freshness metadata model for origin TTLs, validators, `no-cache`, and stale windows.
- Conservative `stale-if-error` support.
- Explicit route policy hints in YAML config.
- Query parameter observation and opt-in query-key hints.
- Process-local disk cache store.
- Dashboard, API, CLI, metrics, documentation, and tests for the new behavior.

## Out of Scope

- Redis or distributed cache coordination.
- Kubernetes operator.
- GraphQL POST response reuse.
- User-specific private caching.
- Unsafe method reuse.
- Reuse of authenticated, cookie-based, `private`, or `no-store` traffic.
- Hosted control plane or required telemetry.

## Design Documents

- [PRD](PRD.md)
  - Product goals, user experience, release scope, non-goals, and success criteria.
- [Architecture Delta](architecture-delta.md)
  - Workspace changes, shared types, proxy flow changes, config model, and failure behavior.
- [Revalidation and Staleness](revalidation-and-staleness.md)
  - Validator metadata, conditional origin requests, `no-cache`, `stale-if-error`, and state transitions.
- [Route Hints and Query Intelligence](route-hints-and-query.md)
  - Route matching, safety boundaries, query parameter observations, and hint config.
- [Disk Store](disk-store.md)
  - Store trait changes, disk entry layout, limits, startup behavior, and corruption handling.
- [Observability and Dashboard](observability-dashboard.md)
  - Metrics, events, API shapes, dashboard pages, and redaction rules.
- [Testing and Release](testing-release.md)
  - Unit, integration, persistence, safety, performance, and release gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone work breakdown with acceptance checks.

## Cross-Cutting Constraints

- Safe default: unknown, risky, stale-without-permission, or unrevalidatable paths go to origin.
- Hard denies remain hard: Authorization, Cookie, unsafe methods, Set-Cookie, `private`, `no-store`, unsupported `Vary`, range requests, and shadow mismatches cannot be relaxed by normal v0.2.0 hints.
- Stale reuse is narrower than fresh reuse: it requires a previously verified safe entry plus origin or route permission.
- Privacy default: do not persist raw Authorization, Cookie, Set-Cookie, request bodies, raw query values in metrics, or observation bodies.
- Local first: disk persistence is process-local and optional; no distributed consistency promises.
- Explainability: every revalidation, stale serve, query-key decision, and disk-store event has stable machine reasons and user-facing messages.
- Fail open: policy, store, validator, dashboard, and metrics failures send traffic to origin unless stale-if-error explicitly applies.

## Milestone Map

- M0: Design and schema preparation
- M1: Freshness metadata and conditional revalidation
- M2: Stale-if-error
- M3: Route hints and query intelligence
- M4: Disk store
- M5: Dashboard, metrics, CLI, and docs
- M6: Release hardening

## Remaining Follow-Ups

- Release artifact and Docker smoke automation.
- `cargo deny check` and `cargo audit` release gates.
- Richer query cardinality and fingerprint-sensitivity analysis.
- Nonblocking disk I/O implementation for high-concurrency disk-store deployments.

Each milestone should preserve the v0.1.0 safety model. A partial v0.2.0 implementation must pass through to origin rather than serving stale, unvalidated, or policy-relaxed responses.
