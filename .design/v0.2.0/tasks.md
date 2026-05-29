# v0.2.0 Implementation Tasks

Status: design draft
Target release: `v0.2.0`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete

## Current Implementation Snapshot

v0.1.0 baseline exists:

- HTTP/1.1 reverse proxy.
- Watch, shadow, and auto modes.
- Conservative hard-deny policy.
- Shadow validation and auto reuse for verified public GET/HEAD responses.
- In-memory cache store.
- Local dashboard, JSON APIs, metrics, admin purge, doctor.
- Release workflow and safety tests.

v0.2.0 starts from that baseline and should keep it working after every milestone.

## M0: Design and Schema Preparation

Goal: add v0.2.0 types and config parsing without changing runtime behavior.

### M0.1 Core Types

- [ ] M0.1.1 Add validator metadata types.
- [ ] M0.1.2 Add freshness metadata types.
- [ ] M0.1.3 Add route hint config structs.
- [ ] M0.1.4 Add query hint config structs.
- [ ] M0.1.5 Add new decision reasons and user messages.

Acceptance:

- Existing tests pass.
- New types serialize/deserialize where needed.
- No runtime behavior changes yet.

### M0.2 Config

- [ ] M0.2.1 Parse `policy.revalidation`.
- [ ] M0.2.2 Parse `policy.stale_if_error`.
- [ ] M0.2.3 Parse `policy.query_intelligence`.
- [ ] M0.2.4 Parse route hints.
- [ ] M0.2.5 Parse disk storage fields.
- [ ] M0.2.6 Validate bounds and conflicts.

Acceptance:

- Invalid hints fail before listeners bind.
- v0.1.0 config remains valid.
- Unknown unsafe config fields do not silently relax policy.

## M1: Freshness Metadata and Conditional Revalidation

Goal: safely revalidate stale eligible entries with origin validators.

### M1.1 Metadata

- [ ] M1.1.1 Extend `CacheEntry` with `fresh_until`, `stale_until`, validators, and `must_revalidate`.
- [ ] M1.1.2 Update memory store for stale entry retrieval.
- [ ] M1.1.3 Parse `ETag` and `Last-Modified`.
- [ ] M1.1.4 Parse origin freshness directives.
- [ ] M1.1.5 Calculate effective freshness from origin and kubio policy.

Acceptance:

- Fresh hit behavior remains unchanged.
- Stale entries are distinguishable from misses.

### M1.2 Conditional Requests

- [ ] M1.2.1 Add conditional headers for stale entries with validators.
- [ ] M1.2.2 Handle 304 by serving stored body.
- [ ] M1.2.3 Merge safe 304 metadata.
- [ ] M1.2.4 Handle 200 by replacing safe stored entry.
- [ ] M1.2.5 Pass through when validators are missing or invalid.

Acceptance:

- ETag and Last-Modified integration tests pass.
- Unsafe 304 metadata does not cause unsafe reuse.

### M1.3 `no-cache`

- [ ] M1.3.1 Change `no-cache` from hard non-store to store-with-revalidation when safe.
- [ ] M1.3.2 Require validators for `no-cache` reuse.
- [ ] M1.3.3 Ensure every `no-cache` use contacts origin.
- [ ] M1.3.4 Update explanations and docs.

Acceptance:

- `no-cache` is never served as a fresh hit.
- `no-cache` without validators is not reused.

## M2: Stale-If-Error

Goal: serve stale verified entries during origin failure only when explicitly allowed and bounded.

### M2.1 Policy

- [ ] M2.1.1 Parse origin `stale-if-error`.
- [ ] M2.1.2 Implement global mode: `disabled`, `origin`, `enabled`.
- [ ] M2.1.3 Implement route-level stale permission.
- [ ] M2.1.4 Calculate `stale_until`.
- [ ] M2.1.5 Reject stale when panic switch is active.

Acceptance:

- Default behavior does not implicitly serve stale.
- Origin/header and route permission paths are tested.

### M2.2 Proxy Flow

- [ ] M2.2.1 Detect revalidation/refresh origin failures.
- [ ] M2.2.2 Serve stale when all gates pass.
- [ ] M2.2.3 Deny stale with explainable reasons when gates fail.
- [ ] M2.2.4 Add debug header `X-Kubio-Status: stale`.
- [ ] M2.2.5 Emit stale served/denied events.

Acceptance:

- Stale serving is bounded by max stale age.
- Protected traffic never receives stale reused responses.

## M3: Route Hints and Query Intelligence

Goal: let operators safely tune known public routes and understand query-key fragmentation.

### M3.1 Route Hints

- [ ] M3.1.1 Implement route hint matcher.
- [ ] M3.1.2 Apply per-route TTL.
- [ ] M3.1.3 Apply per-route stale-if-error cap.
- [ ] M3.1.4 Implement force-protect hint.
- [ ] M3.1.5 Implement sensitive path acknowledgment without overriding hard denies.

Acceptance:

- Hint specificity is deterministic.
- Hard-deny overrides are tested.

### M3.2 Query Hints

- [ ] M3.2.1 Apply `query.ignore`.
- [ ] M3.2.2 Apply `query.include`.
- [ ] M3.2.3 Preserve repeated parameter order.
- [ ] M3.2.4 Record hint applied/rejected reasons.
- [ ] M3.2.5 Test non-matching route behavior.

Acceptance:

- Existing query normalization remains default.
- Configured hints affect only matching routes.

### M3.3 Query Intelligence

- [ ] M3.3.1 Track query parameter names by route.
- [ ] M3.3.2 Track bounded cardinality classes.
- [ ] M3.3.3 Track fingerprint sensitivity.
- [ ] M3.3.4 Generate safe-ignore suggestions.
- [ ] M3.3.5 Redact sensitive query values everywhere.

Acceptance:

- Dashboard/API can show query suggestions.
- Raw query values never appear in metrics or dashboard output.

## M4: Disk Store

Goal: add process-local persistent storage.

### M4.1 Store Implementation

- [ ] M4.1.1 Select disk backend after dependency review.
- [ ] M4.1.2 Implement disk store open/create.
- [ ] M4.1.3 Encode/decode versioned entries.
- [ ] M4.1.4 Implement get/put/purge/stats.
- [ ] M4.1.5 Enforce max size and max object size.

Acceptance:

- Memory remains default.
- Disk store passes existing store trait tests.

### M4.2 Persistence and Recovery

- [ ] M4.2.1 Persist safe entries.
- [ ] M4.2.2 Recover entries on restart.
- [ ] M4.2.3 Drop expired entries on startup or first access.
- [ ] M4.2.4 Skip corrupt entries safely.
- [ ] M4.2.5 Protect Tokio runtime from blocking disk I/O.

Acceptance:

- Safe entry survives restart.
- Corrupt single entry does not crash hot path.

## M5: Dashboard, Metrics, CLI, and Docs

Goal: expose v0.2.0 behavior clearly.

### M5.1 Metrics and Events

- [ ] M5.1.1 Add revalidation counters.
- [ ] M5.1.2 Add stale served/denied counters.
- [ ] M5.1.3 Add hint counters.
- [ ] M5.1.4 Add store error counters.
- [ ] M5.1.5 Add bounded event types.

Acceptance:

- Metrics labels are bounded.
- Sensitive values are absent.

### M5.2 Dashboard APIs and UI

- [ ] M5.2.1 Extend overview API.
- [ ] M5.2.2 Extend route detail API.
- [ ] M5.2.3 Add store API.
- [ ] M5.2.4 Add query param snapshots.
- [ ] M5.2.5 Update dashboard pages.

Acceptance:

- User can inspect revalidation, stale, hints, query suggestions, and store status.

### M5.3 CLI and Docs

- [ ] M5.3.1 Update `kubio routes`.
- [ ] M5.3.2 Update `kubio explain`.
- [ ] M5.3.3 Update `kubio doctor`.
- [ ] M5.3.4 Add v0.2.0 example config.
- [ ] M5.3.5 Update README and docs.
- [ ] M5.3.6 Draft release notes.

Acceptance:

- CLI output explains new reasons.
- Docs state defaults and limits.

## M6: Release Hardening

Goal: ship v0.2.0 with safety, persistence, and performance confidence.

### M6.1 Tests

- [ ] M6.1.1 Add revalidation integration tests.
- [ ] M6.1.2 Add stale-if-error integration tests.
- [ ] M6.1.3 Add route hint tests.
- [ ] M6.1.4 Add query intelligence tests.
- [ ] M6.1.5 Add disk persistence tests.
- [ ] M6.1.6 Add privacy regression tests.

Acceptance:

- Existing v0.1.0 safety tests pass.
- New safety gates are covered.

### M6.2 Performance and Release

- [ ] M6.2.1 Extend local performance smoke script.
- [ ] M6.2.2 Add disk store smoke test.
- [ ] M6.2.3 Add release artifact smoke with v0.2.0 config.
- [ ] M6.2.4 Update Docker image smoke.
- [ ] M6.2.5 Publish release notes.

Acceptance:

- Release artifact runs smoke test.
- Disk store and revalidation paths are exercised before tag.

## Cross-Milestone Safety Tasks

- [ ] S.1 Authorization is never fresh-reused, revalidated-reused, or stale-served.
- [ ] S.2 Cookie traffic is never fresh-reused, revalidated-reused, or stale-served.
- [ ] S.3 Set-Cookie responses are never persisted.
- [ ] S.4 Private/no-store responses are never persisted.
- [ ] S.5 `no-cache` is never served without revalidation.
- [ ] S.6 Unsupported Vary and Vary wildcard remain protected.
- [ ] S.7 Shadow mismatch blocks fresh, revalidated, and stale reuse.
- [ ] S.8 Panic switch blocks fresh, revalidated, and stale reuse.
- [ ] S.9 Raw query values and sensitive headers do not appear in metrics/dashboard/logs/disk metadata.
- [ ] S.10 Disk corruption cannot cause unsafe reuse.

## Suggested Implementation Order

1. M0 types and config.
2. M1 revalidation and `no-cache`.
3. M2 stale-if-error.
4. M3 route hints and query intelligence.
5. M4 disk store.
6. M5 observability and docs.
7. M6 release hardening.

Do not implement stale-if-error before conditional revalidation metadata and hard-deny checks are in place.
