# v0.1.0 Implementation Tasks

Status: complete for v0.1.0 baseline
Target release: `v0.1.0`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete

## Current Implementation Snapshot

Completed baseline:

- Rust workspace, CLI, config loading/validation, telemetry, repository docs, Dockerfile, and CI.
- Local reverse proxy with origin forwarding, gateway errors, hop-by-hop header removal, and graceful Ctrl-C/SIGTERM shutdown.
- Route clustering, cache key hashing, response fingerprinting, observer snapshots, route states, and bounded event storage.
- Conservative policy engine with request/response hard denies, freshness profiles, and user-facing explanations.
- Shadow validation, mismatch demotion, safe auto reuse, memory cache store, purge, debug headers, and panic switch.
- Local dashboard, JSON APIs, configurable metrics endpoint, admin purge API, and CLI admin commands.

Hardening added after the first implementation pass:

- `observability.metrics_path` and `observability.metrics` are honored by the dashboard router.
- `kubio purge` can send `--admin-token` or `KUBIO_ADMIN_TOKEN`.
- `doctor` checks dashboard overview, storage snapshot fields, configured metrics path, and panic-switch state.
- Panic-switch events are emitted on active/inactive transitions instead of every request.
- Active panic switch prevents reuse, storage, and promotion.
- Policy explanations retain concrete oversized/fingerprint reasons.
- Headers named by `Connection` are removed as hop-by-hop headers.
- Config validation rejects zero limits, invalid promotion thresholds, invalid mismatch rates, and invalid metrics paths.

Release-hardening added in the remaining-task pass:

- Added a release workflow for Linux x86_64 binary artifact upload, checksums, and Docker smoke tests.
- Added property tests for arbitrary path, query ordering, and sensitive header redaction invariants.
- Added a local performance smoke script under `examples/bench`.
- Broadened integration coverage for Cookie, Set-Cookie, private/no-cache, Vary wildcard, unsafe method body forwarding, shadow mismatch, origin timeout, panic switch, and sensitive-value exclusion from snapshots/metrics.
- Added streaming pass-through for ineligible/large origin responses and streamed request forwarding for cache misses.

## M0: Project Skeleton

Goal: create a buildable Rust workspace with basic CLI, config, logging, CI, and repository docs.

### M0.1 Workspace

- [x] M0.1.1 Create root `Cargo.toml` workspace.
- [x] M0.1.2 Add crates: `kubio-cli`, `kubio-core`, `kubio-proxy`, `kubio-policy`, `kubio-observe`, `kubio-store`, `kubio-dashboard`, `kubio-telemetry`.
- [x] M0.1.3 Add shared workspace dependency versions.
- [x] M0.1.4 Add minimal crate-level docs.

Acceptance:

- `cargo metadata` succeeds.
- `cargo test --workspace` succeeds with placeholder tests.

### M0.2 CLI Skeleton

- [x] M0.2.1 Add `kubio` binary in `kubio-cli`.
- [x] M0.2.2 Implement subcommands: `serve`, `routes`, `explain`, `doctor`, `purge`.
- [x] M0.2.3 Implement flags for `serve`: `--to`, `--listen`, `--dashboard`, `--mode`, `--config`, `--freshness`, `--debug-headers`, `--panic-file`.
- [x] M0.2.4 Print startup output for `serve`.

Acceptance:

- `kubio --help` works.
- `kubio serve --help` documents defaults.
- Missing `--to` fails clearly.

### M0.3 Config

- [x] M0.3.1 Define config structs and defaults.
- [x] M0.3.2 Parse optional YAML config file.
- [x] M0.3.3 Merge defaults, file, and CLI flags.
- [x] M0.3.4 Validate origin URL, addresses, mode, sizes, and TTLs.
- [x] M0.3.5 Redact secrets for display.

Acceptance:

- Invalid config fails before listeners bind.
- CLI flags override config file values.
- Effective config is serializable for dashboard.

### M0.4 Telemetry Baseline

- [x] M0.4.1 Add structured logging.
- [x] M0.4.2 Add redaction helpers for sensitive headers.
- [x] M0.4.3 Add no-op or in-memory metrics recorder abstraction.
- [x] M0.4.4 Add trace span conventions.

Acceptance:

- Logs do not print sensitive header values in unit tests.
- Crates can record metrics without depending on dashboard.

### M0.5 Repository Setup

- [x] M0.5.1 Add README with first-run demo.
- [x] M0.5.2 Add CONTRIBUTING.
- [x] M0.5.3 Add SECURITY.
- [x] M0.5.4 Add CI workflow for fmt, clippy, tests.
- [x] M0.5.5 Add Apache-2.0 license confirmation.

Acceptance:

- CI runs on pull requests.
- README demo command matches PRD.

## M1: Basic Reverse Proxy

Goal: make `kubio serve --to ...` work as a local HTTP reverse proxy in watch mode.

### M1.1 Server Lifecycle

- [x] M1.1.1 Start Tokio runtime from CLI.
- [x] M1.1.2 Bind proxy listener to configured address.
- [x] M1.1.3 Handle graceful shutdown on SIGINT/SIGTERM.
- [x] M1.1.4 Keep dashboard/metrics failures isolated from proxy.

Acceptance:

- Proxy starts on `0.0.0.0:8080` by default.
- Process exits cleanly on Ctrl-C.

### M1.2 Origin Forwarding

- [x] M1.2.1 Rewrite inbound URI to origin scheme/authority.
- [x] M1.2.2 Preserve method, path, query, body, and relevant headers.
- [x] M1.2.3 Remove hop-by-hop headers.
- [x] M1.2.4 Stream request body to origin.
- [x] M1.2.5 Stream response body to client.
- [x] M1.2.6 Preserve origin status and response headers except hop-by-hop headers.

Acceptance:

- GET, HEAD, POST, query strings, and request bodies forward correctly.
- Origin response body and status are preserved.

### M1.3 Error Handling

- [x] M1.3.1 Add origin connect timeout.
- [x] M1.3.2 Return `502` on origin connection failure.
- [x] M1.3.3 Return `504` on origin timeout.
- [x] M1.3.4 Record origin error metrics/events.

Acceptance:

- Origin unavailable does not crash kubio.
- Error responses are deterministic and tested.

## M2: Observation

Goal: safely collect route-level metadata and fingerprints without serving cached data.

### M2.1 Core Types

- [x] M2.1.1 Define `RouteId`.
- [x] M2.1.2 Define `CacheKeyHash`.
- [x] M2.1.3 Define `Decision`, `DecisionReason`, `RouteState`.
- [x] M2.1.4 Define `ResponseFingerprint`.
- [x] M2.1.5 Define status class and latency snapshot types.

Acceptance:

- Types serialize for dashboard APIs.
- Types have unit tests where normalization is involved.

### M2.2 Route Clustering

- [x] M2.2.1 Normalize numeric path segments.
- [x] M2.2.2 Normalize UUID-like segments.
- [x] M2.2.3 Normalize ULID-like segments.
- [x] M2.2.4 Normalize long hex segments.
- [x] M2.2.5 Exclude query from route id.
- [x] M2.2.6 Add never-panic property test.

Acceptance:

- Example paths from PRD normalize as expected.
- Arbitrary path input does not panic.

### M2.3 Cache Key Hashing

- [x] M2.3.1 Implement query normalization.
- [x] M2.3.2 Preserve repeated parameter order.
- [x] M2.3.3 Preserve all query parameters.
- [x] M2.3.4 Hash full key for storage/observation.
- [x] M2.3.5 Prevent raw cache keys from metrics.

Acceptance:

- Same logical query order yields same key hash.
- Different relevant query values yield different key hashes.

### M2.4 Fingerprinting

- [x] M2.4.1 Hash stable response headers.
- [x] M2.4.2 Exclude volatile headers.
- [x] M2.4.3 Hash body for bounded eligible responses.
- [x] M2.4.4 Skip promotion when body exceeds fingerprint limit.
- [x] M2.4.5 Unit test fingerprint stability.

Acceptance:

- Date/request-id changes do not alter fingerprint.
- Body changes alter fingerprint.

### M2.5 Observer

- [x] M2.5.1 Track route request counts.
- [x] M2.5.2 Track origin request counts.
- [x] M2.5.3 Track protected and bypass counts.
- [x] M2.5.4 Track status class distribution.
- [x] M2.5.5 Track latency distribution.
- [x] M2.5.6 Track repeat frequency by key hash.
- [x] M2.5.7 Add bounded event ring buffer.

Acceptance:

- Watch mode records route stats.
- Observation state stores no sensitive header values.

## M3: Safety Classifier

Goal: protect risky requests/responses and produce explainable decisions.

### M3.1 Request Signals

- [x] M3.1.1 Detect unsafe methods.
- [x] M3.1.2 Detect `Authorization`.
- [x] M3.1.3 Detect `Cookie`.
- [x] M3.1.4 Detect Range requests.
- [x] M3.1.5 Detect GET/HEAD with body.
- [x] M3.1.6 Score sensitive paths.

Acceptance:

- Request hard denies produce `Protect` or `Bypass`.
- Reasons are present and tested.

### M3.2 Response Signals

- [x] M3.2.1 Detect `Set-Cookie`.
- [x] M3.2.2 Parse `Cache-Control: no-store`.
- [x] M3.2.3 Parse `Cache-Control: private`.
- [x] M3.2.4 Parse `Cache-Control: no-cache`.
- [x] M3.2.5 Parse `Vary`.
- [x] M3.2.6 Classify status code cacheability.

Acceptance:

- Safety-critical headers prevent storage/reuse.
- `Vary: *` is never reused.

### M3.3 Policy Engine

- [x] M3.3.1 Implement hard deny evaluation.
- [x] M3.3.2 Implement deterministic score.
- [x] M3.3.3 Implement freshness profile TTL selection.
- [x] M3.3.4 Implement explanation mapping.
- [x] M3.3.5 Return structured `PolicyDecision`.

Acceptance:

- Every decision has at least one reason.
- Policy errors fail closed for reuse and open to origin.

### M3.4 Integration

- [x] M3.4.1 Apply request precheck in proxy.
- [x] M3.4.2 Apply response decision after origin headers.
- [x] M3.4.3 Record policy decisions to observer and metrics.
- [x] M3.4.4 Emit protection events.

Acceptance:

- Auth/cookie/no-store/private/unsafe method tests pass through origin and are marked protected.

## M4: Shadow Validation

Goal: validate whether reuse would have been safe without serving cached data.

### M4.1 Fingerprint History

- [x] M4.1.1 Store latest fingerprint by cache key hash.
- [x] M4.1.2 Bound key history by count and age.
- [x] M4.1.3 Track first seen and last seen timestamps.
- [x] M4.1.4 Record per-route fingerprint stability.

Acceptance:

- Repeated keys can be compared.
- History eviction is deterministic.

### M4.2 Shadow Comparison

- [x] M4.2.1 Compare current fingerprint with previous fingerprint.
- [x] M4.2.2 Increment shadow match count.
- [x] M4.2.3 Increment shadow mismatch count.
- [x] M4.2.4 Emit mismatch demotion event.
- [x] M4.2.5 Exclude hard-denied requests from promotion.

Acceptance:

- Stable origin responses produce matches.
- Changing origin responses produce mismatches.
- Client always receives origin response in shadow mode.

### M4.3 Promotion and Demotion

- [x] M4.3.1 Implement Candidate threshold.
- [x] M4.3.2 Implement 20 validation / 0 mismatch rule.
- [x] M4.3.3 Implement mismatch demotion.
- [x] M4.3.4 Surface route state in observer.
- [x] M4.3.5 Add route state transition tests.

Acceptance:

- Stable public route becomes auto eligible.
- Any mismatch blocks auto eligibility.

## M5: Safe Auto Reuse

Goal: serve reused responses only for verified safe GET/HEAD 200 responses.

### M5.1 Cache Store

- [x] M5.1.1 Define `CacheStore` trait.
- [x] M5.1.2 Implement memory store get/put.
- [x] M5.1.3 Enforce TTL expiration.
- [x] M5.1.4 Enforce max object size.
- [x] M5.1.5 Enforce max total size.
- [x] M5.1.6 Track bytes, entries, and evictions.
- [x] M5.1.7 Implement purge all and purge by route.

Acceptance:

- Expired entries are not served.
- Store limits are tested.

### M5.2 Store Eligible Origin Responses

- [x] M5.2.1 Buffer small eligible responses.
- [x] M5.2.2 Strip hop-by-hop response headers before storage.
- [x] M5.2.3 Store status, headers, body, expiry, route id, key hash, fingerprint.
- [x] M5.2.4 Treat store failure as origin pass-through.
- [x] M5.2.5 Record store events and metrics.

Acceptance:

- Ineligible responses are not stored.
- Store errors do not affect client response.

### M5.3 Serve Reused Responses

- [x] M5.3.1 Check request hard denies before lookup.
- [x] M5.3.2 Check panic switch before lookup.
- [x] M5.3.3 Lookup only auto-eligible route/key.
- [x] M5.3.4 Serve only fresh entries.
- [x] M5.3.5 Add optional `X-Kubio-Status` debug header.
- [x] M5.3.6 Record reuse metrics.

Acceptance:

- Verified stable GET can be reused.
- Protected traffic always goes to origin.
- Debug headers are disabled by default.

### M5.4 Panic Switch

- [x] M5.4.1 Implement `--panic-file`.
- [x] M5.4.2 Check file existence on each request or with short cached interval.
- [x] M5.4.3 Emit enabled/disabled transition events.
- [x] M5.4.4 Ensure active panic switch prevents reuse.

Acceptance:

- Creating panic file immediately stops reuse.
- Removing panic file allows policy-controlled reuse again.

## M6: Dashboard, Metrics, Docs, Release

Goal: make the release understandable, observable, packaged, and documented.

### M6.1 Metrics

- [x] M6.1.1 Expose `/metrics`.
- [x] M6.1.2 Implement required counters.
- [x] M6.1.3 Implement cache gauges.
- [x] M6.1.4 Implement request/origin duration histograms.
- [x] M6.1.5 Enforce allowed labels.

Acceptance:

- Prometheus scrape returns required metrics.
- Sensitive values never appear in metrics.

### M6.2 Dashboard APIs

- [x] M6.2.1 Implement `GET /api/overview`.
- [x] M6.2.2 Implement `GET /api/routes`.
- [x] M6.2.3 Implement route detail by route hash.
- [x] M6.2.4 Implement `GET /api/events`.
- [x] M6.2.5 Implement `GET /api/config`.
- [x] M6.2.6 Implement optional `POST /api/purge`.

Acceptance:

- APIs return redacted snapshots.
- Dashboard failure does not affect proxy.

### M6.3 Dashboard UI

- [x] M6.3.1 Build Overview page.
- [x] M6.3.2 Build Routes page.
- [x] M6.3.3 Build Route Detail page.
- [x] M6.3.4 Build Events page.
- [x] M6.3.5 Build Config page.
- [x] M6.3.6 Use product language from design docs.

Acceptance:

- A user can see candidates, protected routes, auto routes, and reasons.
- UI does not expose sensitive values.

### M6.4 CLI Admin Commands

- [x] M6.4.1 Implement `kubio routes` through local API.
- [x] M6.4.2 Implement `kubio explain`.
- [x] M6.4.3 Implement `kubio purge --all`.
- [x] M6.4.4 Implement `kubio purge --route`.
- [x] M6.4.5 Implement `kubio doctor`.

Acceptance:

- Commands fail clearly when no running kubio admin API is available.
- `doctor` checks config, origin, dashboard, metrics, storage, and panic switch.

### M6.5 Documentation

- [x] M6.5.1 Write `docs/getting-started.md`.
- [x] M6.5.2 Write `docs/configuration.md`.
- [x] M6.5.3 Write `docs/how-kubio-decides.md`.
- [x] M6.5.4 Write `docs/safety-model.md`.
- [x] M6.5.5 Write `docs/metrics.md`.
- [x] M6.5.6 Write `docs/deployment.md`.
- [x] M6.5.7 Write `docs/development.md`.
- [x] M6.5.8 Write `docs/roadmap.md`.

Acceptance:

- README local demo works in under 5 minutes.
- Safety limitations are documented.

### M6.6 Release Engineering

- [x] M6.6.1 Add Dockerfile.
- [x] M6.6.2 Add release build workflow.
- [x] M6.6.3 Generate checksums.
- [x] M6.6.4 Build Linux x86_64 binary.
- [x] M6.6.5 Build Docker image.
- [x] M6.6.6 Draft release notes.

Acceptance:

- Release artifact runs smoke test.
- Release notes document known limitations.

## Cross-Milestone Safety Tasks

- [x] S.1 Add integration test proving Authorization is never reused.
- [x] S.2 Add integration test proving Cookie is never reused.
- [x] S.3 Add integration test proving Set-Cookie response is never stored.
- [x] S.4 Add integration test proving no-store/private/no-cache are not reused.
- [x] S.5 Add integration test proving Vary wildcard is not reused.
- [x] S.6 Add integration test proving shadow mismatch blocks auto.
- [x] S.7 Add test proving panic switch stops reuse.
- [x] S.8 Add test proving sensitive header values do not appear in logs.
- [x] S.9 Add test proving sensitive header values do not appear in metrics.
- [x] S.10 Add test proving sensitive header values do not appear in dashboard APIs.

## Suggested Implementation Order

1. M0 workspace, CLI, config, telemetry baseline.
2. M1 reverse proxy.
3. M2 route clustering, cache key hashing, observer, fingerprints.
4. M3 policy engine and safety integration.
5. M4 shadow validation.
6. M5 memory store and safe auto reuse.
7. M6 dashboard, metrics, docs, release packaging.

Do not start auto reuse before M3 hard-deny rules and M4 mismatch handling are implemented and tested.
