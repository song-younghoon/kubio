# Testing and Release

Status: implemented design reference
Target release: `v0.1.0`

## Goals

Testing must prove that kubio is safe before it is fast. The release should be blocked by any regression that can serve a protected or unstable response from cache.

## Test Layers

Use five layers:

1. Unit tests for deterministic logic.
2. Integration tests with a local origin server.
3. Property tests for normalization and safety invariants.
4. Performance tests for proxy overhead.
5. Release smoke tests from packaged artifacts.

## Unit Tests

Required areas:

- CLI parsing.
- Config defaults and merge precedence.
- Config validation failures.
- Cache-Control parsing.
- Vary parsing and allowlist behavior.
- Authorization protection.
- Cookie protection.
- Set-Cookie protection.
- no-store/private/no-cache behavior.
- Query normalization.
- Route clustering.
- Sensitive path detection.
- Cache key hashing.
- Fingerprint generation.
- Volatile header exclusion.
- Decision reasons.
- Freshness profile selection.
- Memory store expiration.
- Memory store eviction.
- Metrics label redaction helpers.

Unit tests should live near the crate that owns the behavior.

## Integration Test Harness

Create a test harness that starts:

- A local origin server with programmable routes.
- A kubio proxy instance bound to an ephemeral port.
- Optional dashboard/admin server.

Origin route behaviors:

- Static public JSON.
- Incrementing JSON.
- Authenticated echo.
- Cookie response.
- no-store response.
- private response.
- no-cache response.
- Vary response.
- Large response.
- Slow response.
- Origin error/timeout.

Integration tests should send real HTTP requests through kubio and assert client-visible behavior, metrics, and observation snapshots.

## Required Integration Scenarios

### Reverse Proxy

- Preserves method, path, query, body, status, headers, and body.
- Removes hop-by-hop headers.
- Returns `502` for origin connection error.
- Returns `504` for origin timeout.

### Watch Mode

- No cached response is served.
- All requests go to origin.
- Route observations are recorded.
- Protected reasons are visible.

### Shadow Mode

- No cached response is served.
- Stable repeated response records shadow matches.
- Unstable repeated response records shadow mismatches.
- Shadow mismatch prevents auto eligibility.

### Auto Mode

- Stable public GET 200 can be reused after validation.
- POST is never reused.
- GET with Authorization always goes to origin.
- GET with Cookie always goes to origin.
- Response with Set-Cookie is not stored.
- Response with `Cache-Control: no-store` is not stored.
- Response with `Cache-Control: private` is not stored.
- Response with `Cache-Control: no-cache` is not reused.
- `Vary: Accept-Encoding` is keyed by `Accept-Encoding`.
- `Vary: *` is not reused.
- Store error causes origin pass-through.
- Panic switch disables reuse immediately.
- Expired entry causes origin pass-through.

### Dashboard and Metrics

- Dashboard binds to localhost by default.
- `/api/overview` and `/api/routes` return expected snapshots.
- `/metrics` exposes required metrics.
- Configured metrics paths expose metrics when enabled and return 404 when disabled.
- Metrics labels do not include raw path, query, or header values.
- Dashboard never displays sensitive header values.

## Property Tests

Use `proptest` for:

- Query normalization is stable across parameter ordering.
- Repeated query parameters preserve relative order.
- Cache key hash changes when Vary-selected header values change.
- Route clustering never panics for arbitrary paths.
- Sensitive header redaction never returns the original secret.
- Metrics label sanitizer rejects or normalizes high-cardinality raw inputs.

## Performance Tests

Initial targets:

- Pass-through p95 overhead <= 5ms in local 100 RPS test.
- Cache-hit p95 overhead <= 2ms in local 100 RPS test.
- Default idle memory <= 100MiB.
- Memory store lookup expected O(1).
- Dashboard polling does not materially change proxy p95.

Scenarios:

- 100 RPS for 10 minutes, 1KiB JSON responses.
- 500 RPS for 10 minutes, 10KiB JSON responses.
- 100 RPS for 10 minutes, 1MiB responses.
- Mixed traffic: 70% GET, 20% POST, 10% authenticated GET.

Performance tests can start as ignored tests or scripts under `examples/bench` and become CI-gated once stable.

## Security and Privacy Tests

Required assertions:

- No raw `Authorization` value appears in logs.
- No raw `Cookie` value appears in logs.
- No raw `Set-Cookie` value appears in logs.
- No sensitive header value appears in dashboard JSON.
- No sensitive header value appears in metrics output.
- Request bodies are not stored in observation state.
- Watch mode stores no response body cache entries.
- Public dashboard binding requires explicit configuration.

## CI Gates

Required before v0.1.0 release:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check
cargo audit
```

If `cargo audit` or `cargo deny` are not adopted immediately, create tracked issues and document the temporary exception before release.

## Release Artifacts

v0.1.0 should publish:

- Source tag.
- Linux x86_64 binary.
- macOS arm64 binary if CI support is available.
- Docker image.
- Checksums.
- Release notes.

## Smoke Test

Release smoke test:

```bash
kubio --help
kubio serve --to http://localhost:3000
curl http://localhost:8080
curl http://127.0.0.1:9900/api/overview
curl http://127.0.0.1:9900/metrics
```

Expected:

- CLI help prints.
- Proxy starts in watch mode.
- Request reaches origin.
- Dashboard API responds.
- Metrics endpoint responds.

## Documentation Gates

Required docs before release:

- `README.md`
- `CONTRIBUTING.md`
- `SECURITY.md`
- `docs/getting-started.md`
- `docs/configuration.md`
- `docs/how-kubio-decides.md`
- `docs/safety-model.md`
- `docs/metrics.md`
- `docs/deployment.md`
- `docs/development.md`
- `docs/roadmap.md`

The README demo must work locally in under 5 minutes from a clean checkout and released binary.

## Release Exit Criteria

All must be true:

- `kubio serve --to http://localhost:3000` works as a local reverse proxy.
- Watch mode observes traffic without behavior change.
- Dashboard shows route-level status and reasons.
- Safety classifier protects Authorization, Cookie, Set-Cookie, no-store, private, no-cache, unsafe methods, and unsupported Vary.
- Shadow validation distinguishes stable and unstable endpoints.
- Auto mode reuses only verified safe GET/HEAD 200 responses.
- Prometheus-compatible metrics are available.
- Panic switch prevents cache reuse.
- Panic switch prevents reuse, storage, and promotion while active.
- No sensitive header values appear in logs, metrics, or dashboard.
- Safety integration tests pass.
