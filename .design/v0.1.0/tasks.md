# v0.1.0 Implementation Tasks

Status: draft
Target release: `v0.1.0`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete

## M0: Project Skeleton

Goal: create a buildable Rust workspace with basic CLI, config, logging, CI, and repository docs.

### M0.1 Workspace

- [ ] M0.1.1 Create root `Cargo.toml` workspace.
- [ ] M0.1.2 Add crates: `kubio-cli`, `kubio-core`, `kubio-proxy`, `kubio-policy`, `kubio-observe`, `kubio-store`, `kubio-dashboard`, `kubio-telemetry`.
- [ ] M0.1.3 Add shared workspace dependency versions.
- [ ] M0.1.4 Add minimal crate-level docs.

Acceptance:

- `cargo metadata` succeeds.
- `cargo test --workspace` succeeds with placeholder tests.

### M0.2 CLI Skeleton

- [ ] M0.2.1 Add `kubio` binary in `kubio-cli`.
- [ ] M0.2.2 Implement subcommands: `serve`, `routes`, `explain`, `doctor`, `purge`.
- [ ] M0.2.3 Implement flags for `serve`: `--to`, `--listen`, `--dashboard`, `--mode`, `--config`, `--freshness`, `--debug-headers`, `--panic-file`.
- [ ] M0.2.4 Print startup output for `serve`.

Acceptance:

- `kubio --help` works.
- `kubio serve --help` documents defaults.
- Missing `--to` fails clearly.

### M0.3 Config

- [ ] M0.3.1 Define config structs and defaults.
- [ ] M0.3.2 Parse optional YAML config file.
- [ ] M0.3.3 Merge defaults, file, and CLI flags.
- [ ] M0.3.4 Validate origin URL, addresses, mode, sizes, and TTLs.
- [ ] M0.3.5 Redact secrets for display.

Acceptance:

- Invalid config fails before listeners bind.
- CLI flags override config file values.
- Effective config is serializable for dashboard.

### M0.4 Telemetry Baseline

- [ ] M0.4.1 Add structured logging.
- [ ] M0.4.2 Add redaction helpers for sensitive headers.
- [ ] M0.4.3 Add no-op or in-memory metrics recorder abstraction.
- [ ] M0.4.4 Add trace span conventions.

Acceptance:

- Logs do not print sensitive header values in unit tests.
- Crates can record metrics without depending on dashboard.

### M0.5 Repository Setup

- [ ] M0.5.1 Add README with first-run demo.
- [ ] M0.5.2 Add CONTRIBUTING.
- [ ] M0.5.3 Add SECURITY.
- [ ] M0.5.4 Add CI workflow for fmt, clippy, tests.
- [ ] M0.5.5 Add Apache-2.0 license confirmation.

Acceptance:

- CI runs on pull requests.
- README demo command matches PRD.

## M1: Basic Reverse Proxy

Goal: make `kubio serve --to ...` work as a local HTTP reverse proxy in watch mode.

### M1.1 Server Lifecycle

- [ ] M1.1.1 Start Tokio runtime from CLI.
- [ ] M1.1.2 Bind proxy listener to configured address.
- [ ] M1.1.3 Handle graceful shutdown on SIGINT/SIGTERM.
- [ ] M1.1.4 Keep dashboard/metrics failures isolated from proxy.

Acceptance:

- Proxy starts on `0.0.0.0:8080` by default.
- Process exits cleanly on Ctrl-C.

### M1.2 Origin Forwarding

- [ ] M1.2.1 Rewrite inbound URI to origin scheme/authority.
- [ ] M1.2.2 Preserve method, path, query, body, and relevant headers.
- [ ] M1.2.3 Remove hop-by-hop headers.
- [ ] M1.2.4 Stream request body to origin.
- [ ] M1.2.5 Stream response body to client.
- [ ] M1.2.6 Preserve origin status and response headers except hop-by-hop headers.

Acceptance:

- GET, HEAD, POST, query strings, and request bodies forward correctly.
- Origin response body and status are preserved.

### M1.3 Error Handling

- [ ] M1.3.1 Add origin connect timeout.
- [ ] M1.3.2 Return `502` on origin connection failure.
- [ ] M1.3.3 Return `504` on origin timeout.
- [ ] M1.3.4 Record origin error metrics/events.

Acceptance:

- Origin unavailable does not crash kubio.
- Error responses are deterministic and tested.

## M2: Observation

Goal: safely collect route-level metadata and fingerprints without serving cached data.

### M2.1 Core Types

- [ ] M2.1.1 Define `RouteId`.
- [ ] M2.1.2 Define `CacheKeyHash`.
- [ ] M2.1.3 Define `Decision`, `DecisionReason`, `RouteState`.
- [ ] M2.1.4 Define `ResponseFingerprint`.
- [ ] M2.1.5 Define status class and latency snapshot types.

Acceptance:

- Types serialize for dashboard APIs.
- Types have unit tests where normalization is involved.

### M2.2 Route Clustering

- [ ] M2.2.1 Normalize numeric path segments.
- [ ] M2.2.2 Normalize UUID-like segments.
- [ ] M2.2.3 Normalize ULID-like segments.
- [ ] M2.2.4 Normalize long hex segments.
- [ ] M2.2.5 Exclude query from route id.
- [ ] M2.2.6 Add never-panic property test.

Acceptance:

- Example paths from PRD normalize as expected.
- Arbitrary path input does not panic.

### M2.3 Cache Key Hashing

- [ ] M2.3.1 Implement query normalization.
- [ ] M2.3.2 Preserve repeated parameter order.
- [ ] M2.3.3 Preserve all query parameters.
- [ ] M2.3.4 Hash full key for storage/observation.
- [ ] M2.3.5 Prevent raw cache keys from metrics.

Acceptance:

- Same logical query order yields same key hash.
- Different relevant query values yield different key hashes.

### M2.4 Fingerprinting

- [ ] M2.4.1 Hash stable response headers.
- [ ] M2.4.2 Exclude volatile headers.
- [ ] M2.4.3 Hash body for bounded eligible responses.
- [ ] M2.4.4 Skip promotion when body exceeds fingerprint limit.
- [ ] M2.4.5 Unit test fingerprint stability.

Acceptance:

- Date/request-id changes do not alter fingerprint.
- Body changes alter fingerprint.

### M2.5 Observer

- [ ] M2.5.1 Track route request counts.
- [ ] M2.5.2 Track origin request counts.
- [ ] M2.5.3 Track protected and bypass counts.
- [ ] M2.5.4 Track status class distribution.
- [ ] M2.5.5 Track latency distribution.
- [ ] M2.5.6 Track repeat frequency by key hash.
- [ ] M2.5.7 Add bounded event ring buffer.

Acceptance:

- Watch mode records route stats.
- Observation state stores no sensitive header values.

## M3: Safety Classifier

Goal: protect risky requests/responses and produce explainable decisions.

### M3.1 Request Signals

- [ ] M3.1.1 Detect unsafe methods.
- [ ] M3.1.2 Detect `Authorization`.
- [ ] M3.1.3 Detect `Cookie`.
- [ ] M3.1.4 Detect Range requests.
- [ ] M3.1.5 Detect GET/HEAD with body.
- [ ] M3.1.6 Score sensitive paths.

Acceptance:

- Request hard denies produce `Protect` or `Bypass`.
- Reasons are present and tested.

### M3.2 Response Signals

- [ ] M3.2.1 Detect `Set-Cookie`.
- [ ] M3.2.2 Parse `Cache-Control: no-store`.
- [ ] M3.2.3 Parse `Cache-Control: private`.
- [ ] M3.2.4 Parse `Cache-Control: no-cache`.
- [ ] M3.2.5 Parse `Vary`.
- [ ] M3.2.6 Classify status code cacheability.

Acceptance:

- Safety-critical headers prevent storage/reuse.
- `Vary: *` is never reused.

### M3.3 Policy Engine

- [ ] M3.3.1 Implement hard deny evaluation.
- [ ] M3.3.2 Implement deterministic score.
- [ ] M3.3.3 Implement freshness profile TTL selection.
- [ ] M3.3.4 Implement explanation mapping.
- [ ] M3.3.5 Return structured `PolicyDecision`.

Acceptance:

- Every decision has at least one reason.
- Policy errors fail closed for reuse and open to origin.

### M3.4 Integration

- [ ] M3.4.1 Apply request precheck in proxy.
- [ ] M3.4.2 Apply response decision after origin headers.
- [ ] M3.4.3 Record policy decisions to observer and metrics.
- [ ] M3.4.4 Emit protection events.

Acceptance:

- Auth/cookie/no-store/private/unsafe method tests pass through origin and are marked protected.

## M4: Shadow Validation

Goal: validate whether reuse would have been safe without serving cached data.

### M4.1 Fingerprint History

- [ ] M4.1.1 Store latest fingerprint by cache key hash.
- [ ] M4.1.2 Bound key history by count and age.
- [ ] M4.1.3 Track first seen and last seen timestamps.
- [ ] M4.1.4 Record per-route fingerprint stability.

Acceptance:

- Repeated keys can be compared.
- History eviction is deterministic.

### M4.2 Shadow Comparison

- [ ] M4.2.1 Compare current fingerprint with previous fingerprint.
- [ ] M4.2.2 Increment shadow match count.
- [ ] M4.2.3 Increment shadow mismatch count.
- [ ] M4.2.4 Emit mismatch demotion event.
- [ ] M4.2.5 Exclude hard-denied requests from promotion.

Acceptance:

- Stable origin responses produce matches.
- Changing origin responses produce mismatches.
- Client always receives origin response in shadow mode.

### M4.3 Promotion and Demotion

- [ ] M4.3.1 Implement Candidate threshold.
- [ ] M4.3.2 Implement 20 validation / 0 mismatch rule.
- [ ] M4.3.3 Implement mismatch demotion.
- [ ] M4.3.4 Surface route state in observer.
- [ ] M4.3.5 Add route state transition tests.

Acceptance:

- Stable public route becomes auto eligible.
- Any mismatch blocks auto eligibility.

## M5: Safe Auto Reuse

Goal: serve reused responses only for verified safe GET/HEAD 200 responses.

### M5.1 Cache Store

- [ ] M5.1.1 Define `CacheStore` trait.
- [ ] M5.1.2 Implement memory store get/put.
- [ ] M5.1.3 Enforce TTL expiration.
- [ ] M5.1.4 Enforce max object size.
- [ ] M5.1.5 Enforce max total size.
- [ ] M5.1.6 Track bytes, entries, and evictions.
- [ ] M5.1.7 Implement purge all and purge by route.

Acceptance:

- Expired entries are not served.
- Store limits are tested.

### M5.2 Store Eligible Origin Responses

- [ ] M5.2.1 Buffer small eligible responses.
- [ ] M5.2.2 Strip hop-by-hop response headers before storage.
- [ ] M5.2.3 Store status, headers, body, expiry, route id, key hash, fingerprint.
- [ ] M5.2.4 Treat store failure as origin pass-through.
- [ ] M5.2.5 Record store events and metrics.

Acceptance:

- Ineligible responses are not stored.
- Store errors do not affect client response.

### M5.3 Serve Reused Responses

- [ ] M5.3.1 Check request hard denies before lookup.
- [ ] M5.3.2 Check panic switch before lookup.
- [ ] M5.3.3 Lookup only auto-eligible route/key.
- [ ] M5.3.4 Serve only fresh entries.
- [ ] M5.3.5 Add optional `X-Kubio-Status` debug header.
- [ ] M5.3.6 Record reuse metrics.

Acceptance:

- Verified stable GET can be reused.
- Protected traffic always goes to origin.
- Debug headers are disabled by default.

### M5.4 Panic Switch

- [ ] M5.4.1 Implement `--panic-file`.
- [ ] M5.4.2 Check file existence on each request or with short cached interval.
- [ ] M5.4.3 Emit enabled/disabled transition events.
- [ ] M5.4.4 Ensure active panic switch prevents reuse.

Acceptance:

- Creating panic file immediately stops reuse.
- Removing panic file allows policy-controlled reuse again.

## M6: Dashboard, Metrics, Docs, Release

Goal: make the release understandable, observable, packaged, and documented.

### M6.1 Metrics

- [ ] M6.1.1 Expose `/metrics`.
- [ ] M6.1.2 Implement required counters.
- [ ] M6.1.3 Implement cache gauges.
- [ ] M6.1.4 Implement request/origin duration histograms.
- [ ] M6.1.5 Enforce allowed labels.

Acceptance:

- Prometheus scrape returns required metrics.
- Sensitive values never appear in metrics.

### M6.2 Dashboard APIs

- [ ] M6.2.1 Implement `GET /api/overview`.
- [ ] M6.2.2 Implement `GET /api/routes`.
- [ ] M6.2.3 Implement route detail by route hash.
- [ ] M6.2.4 Implement `GET /api/events`.
- [ ] M6.2.5 Implement `GET /api/config`.
- [ ] M6.2.6 Implement optional `POST /api/purge`.

Acceptance:

- APIs return redacted snapshots.
- Dashboard failure does not affect proxy.

### M6.3 Dashboard UI

- [ ] M6.3.1 Build Overview page.
- [ ] M6.3.2 Build Routes page.
- [ ] M6.3.3 Build Route Detail page.
- [ ] M6.3.4 Build Events page.
- [ ] M6.3.5 Build Config page.
- [ ] M6.3.6 Use product language from design docs.

Acceptance:

- A user can see candidates, protected routes, auto routes, and reasons.
- UI does not expose sensitive values.

### M6.4 CLI Admin Commands

- [ ] M6.4.1 Implement `kubio routes` through local API.
- [ ] M6.4.2 Implement `kubio explain`.
- [ ] M6.4.3 Implement `kubio purge --all`.
- [ ] M6.4.4 Implement `kubio purge --route`.
- [ ] M6.4.5 Implement `kubio doctor`.

Acceptance:

- Commands fail clearly when no running kubio admin API is available.
- `doctor` checks config, origin, dashboard, metrics, storage, and panic switch.

### M6.5 Documentation

- [ ] M6.5.1 Write `docs/getting-started.md`.
- [ ] M6.5.2 Write `docs/configuration.md`.
- [ ] M6.5.3 Write `docs/how-kubio-decides.md`.
- [ ] M6.5.4 Write `docs/safety-model.md`.
- [ ] M6.5.5 Write `docs/metrics.md`.
- [ ] M6.5.6 Write `docs/deployment.md`.
- [ ] M6.5.7 Write `docs/development.md`.
- [ ] M6.5.8 Write `docs/roadmap.md`.

Acceptance:

- README local demo works in under 5 minutes.
- Safety limitations are documented.

### M6.6 Release Engineering

- [ ] M6.6.1 Add Dockerfile.
- [ ] M6.6.2 Add release build workflow.
- [ ] M6.6.3 Generate checksums.
- [ ] M6.6.4 Build Linux x86_64 binary.
- [ ] M6.6.5 Build Docker image.
- [ ] M6.6.6 Draft release notes.

Acceptance:

- Release artifact runs smoke test.
- Release notes document known limitations.

## Cross-Milestone Safety Tasks

- [ ] S.1 Add integration test proving Authorization is never reused.
- [ ] S.2 Add integration test proving Cookie is never reused.
- [ ] S.3 Add integration test proving Set-Cookie response is never stored.
- [ ] S.4 Add integration test proving no-store/private/no-cache are not reused.
- [ ] S.5 Add integration test proving Vary wildcard is not reused.
- [ ] S.6 Add integration test proving shadow mismatch blocks auto.
- [ ] S.7 Add test proving panic switch stops reuse.
- [ ] S.8 Add test proving sensitive header values do not appear in logs.
- [ ] S.9 Add test proving sensitive header values do not appear in metrics.
- [ ] S.10 Add test proving sensitive header values do not appear in dashboard APIs.

## Suggested Implementation Order

1. M0 workspace, CLI, config, telemetry baseline.
2. M1 reverse proxy.
3. M2 route clustering, cache key hashing, observer, fingerprints.
4. M3 policy engine and safety integration.
5. M4 shadow validation.
6. M5 memory store and safe auto reuse.
7. M6 dashboard, metrics, docs, release packaging.

Do not start auto reuse before M3 hard-deny rules and M4 mismatch handling are implemented and tested.
