# v0.3.1 Implementation Tasks

Status: implemented
Target release: `v0.3.1`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.3.1 scope

## Current Implementation Snapshot

v0.3.0 shipped:

- HTTP/1.1 and HTTP/2 reverse proxy runtime.
- HTTP/3 config parsing and guarded startup failure.
- Protocol-aware metrics, dashboard fields, CLI output, and debug headers.
- Local benchmark and baseline scenario smoke scripts.

v0.3.1 targets the deferred HTTP/3 runtime and release benchmark work.

## M0: Design and Dependency Review

Goal: lock the v0.3.1 architecture before changing runtime behavior.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.3.1 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add architecture delta.
- [x] M0.1.4 Add HTTP/3 runtime design.
- [x] M0.1.5 Add performance and benchmark design.
- [x] M0.1.6 Add observability design.
- [x] M0.1.7 Add testing and release design.
- [x] M0.1.8 Add implementation task breakdown.

Acceptance:

- Roadmap links v0.3.1.
- v0.3.1 scope explicitly includes downstream HTTP/3, upstream HTTP/3 experiment, and release budgets.

### M0.2 Dependency Review

- [x] M0.2.1 Review `h3` server API.
- [x] M0.2.2 Review `h3-quinn` integration API.
- [x] M0.2.3 Review Quinn server config, limits, and UDP socket setup.
- [x] M0.2.4 Review rustls version alignment across tokio-rustls and Quinn.
- [x] M0.2.5 Decide direct h3 upstream client versus reqwest unstable HTTP/3.
- [x] M0.2.6 Update supply-chain deny/audit allowlists if needed.

Acceptance:

- Dependency versions are pinned or bounded.
- Build feature strategy is documented.
- HTTP/3 dependencies are absent from non-HTTP/3 builds where practical.

## M1: Transport Boundary and Feature Gates

Goal: make room for HTTP/3 without entangling policy/cache with QUIC.

### M1.1 Workspace Structure

- [x] M1.1.1 Add `crates/kubio-transport`.
- [x] M1.1.2 Move TCP/TLS listener setup behind transport module APIs.
- [x] M1.1.3 Move origin client facade behind transport module APIs.
- [x] M1.1.4 Keep existing HTTP/1.1 and HTTP/2 behavior passing.
- [x] M1.1.5 Add `experimental-http3` feature plumbing.

Acceptance:

- Default build still runs h1/h2 tests.
- Feature-enabled build compiles with HTTP/3 dependencies.
- Policy/cache crates do not depend on `h3`, `h3-quinn`, or `quinn`.

### M1.2 Config Validation

- [x] M1.2.1 Add HTTP/3 authority allowlist config.
- [x] M1.2.2 Add QPACK and UDP payload limit config.
- [x] M1.2.3 Validate HTTP/3 feature availability.
- [x] M1.2.4 Validate TLS requirement.
- [x] M1.2.5 Validate Alt-Svc requirements.
- [x] M1.2.6 Validate upstream HTTP/3 experiment requirements.

Acceptance:

- Invalid HTTP/3 config fails before listeners bind.
- Existing v0.3.0 guarded configs fail clearly when feature is absent.

## M2: Downstream HTTP/3 Runtime

Goal: accept HTTP/3 client requests over QUIC.

### M2.1 QUIC Endpoint

- [x] M2.1.1 Load TLS cert/key into Quinn server config.
- [x] M2.1.2 Set ALPN to `h3`.
- [x] M2.1.3 Bind UDP listener.
- [x] M2.1.4 Apply idle, stream, field-section, QPACK, and UDP payload limits.
- [x] M2.1.5 Disable 0-RTT.
- [x] M2.1.6 Add graceful shutdown.

Acceptance:

- HTTP/3 listener starts only when enabled.
- UDP bind failure fails startup when HTTP/3 is enabled.

### M2.2 h3 Request Adapter

- [x] M2.2.1 Accept h3 requests from Quinn connections.
- [x] M2.2.2 Normalize pseudo headers.
- [x] M2.2.3 Reject malformed requests before policy/cache.
- [x] M2.2.4 Bridge streaming request bodies into proxy body type.
- [x] M2.2.5 Record downstream protocol as HTTP/3.

Acceptance:

- Safe GET reaches the existing policy/cache handler.
- Malformed HTTP/3 requests have no cache effect.

### M2.3 h3 Response Writer

- [x] M2.3.1 Write response headers.
- [x] M2.3.2 Stream response body.
- [x] M2.3.3 Handle client disconnects.
- [x] M2.3.4 Preserve debug headers.
- [x] M2.3.5 Record bounded write errors.

Acceptance:

- Large unstoreable HTTP/3 responses stream without full buffering.
- No partial response is stored.

### M2.4 HTTP/3 Safety Tests

- [x] M2.4.1 Safe GET reuse over HTTP/3.
- [x] M2.4.2 Authorization/Cookie protection over HTTP/3.
- [x] M2.4.3 Response hard-deny protection over HTTP/3.
- [x] M2.4.4 Revalidation over HTTP/3.
- [x] M2.4.5 Stale-if-error over HTTP/3.
- [x] M2.4.6 Cross-protocol cache-key equivalence.

Acceptance:

- HTTP/3 does not change reuse safety.
- Hard denies remain hard.

## M3: Alt-Svc

Goal: advertise HTTP/3 safely only when kubio is authoritative for the request authority.

- [x] M3.1 Emit Alt-Svc for configured authorities.
- [x] M3.2 Skip Alt-Svc for unconfigured authorities.
- [x] M3.3 Skip Alt-Svc for dashboard/admin responses.
- [x] M3.4 Add bounded skip reasons.
- [x] M3.5 Add metrics/events.
- [x] M3.6 Add tests for exact authority matching.

Acceptance:

- Alt-Svc is opt-in.
- Alt-Svc is not emitted for arbitrary Host or `:authority`.

## M4: Upstream HTTP/3 Experiment

Goal: try HTTP/3 to capable origins with deterministic fallback.

### M4.1 Client Implementation

- [x] M4.1.1 Decide direct h3/Quinn client or reqwest unstable HTTP/3 implementation.
- [x] M4.1.2 Implement HTTPS-origin-only HTTP/3 attempt path.
- [x] M4.1.3 Add connection pooling or reuse.
- [x] M4.1.4 Apply origin timeout and idle settings.
- [x] M4.1.5 Record attempted and final upstream protocol.

Acceptance:

- Upstream HTTP/3 can be enabled and disabled cleanly.
- Origin protocol labels remain bounded.

### M4.2 Fallback

- [x] M4.2.1 Retry replayable failures to HTTP/2 or HTTP/1.1 when configured.
- [x] M4.2.2 Block unsafe/non-replayable fallback after body streaming can have occurred.
- [x] M4.2.3 Return bounded gateway error when required HTTP/3 fails.
- [x] M4.2.4 Add fallback metrics/events.

Acceptance:

- Fallback behavior is deterministic and visible.
- Cache keys do not include upstream protocol.

### M4.3 Upstream Tests

- [x] M4.3.1 HTTP/3 origin success.
- [x] M4.3.2 HTTP/3 origin required failure.
- [x] M4.3.3 HTTP/3 origin preferred fallback.
- [x] M4.3.4 Unsafe/non-replayable fallback rejection.
- [x] M4.3.5 Revalidation through upstream HTTP/3.

Acceptance:

- Upstream HTTP/3 is tested behind explicit feature/config.

## M5: Observability, CLI, Docs, and Examples

Goal: make HTTP/3 behavior inspectable and safe to operate.

- [x] M5.1 Add HTTP/3 connection and stream metrics.
- [x] M5.2 Add QUIC handshake failure metrics.
- [x] M5.3 Add Alt-Svc metrics/events.
- [x] M5.4 Add upstream HTTP/3 attempt/fallback metrics.
- [x] M5.5 Extend dashboard overview and route detail.
- [x] M5.6 Extend `kubio doctor`.
- [x] M5.7 Extend `kubio routes` and `kubio explain`.
- [x] M5.8 Add HTTP/3 runtime example config.
- [x] M5.9 Update configuration, deployment, metrics, and README docs.

Acceptance:

- CLI distinguishes build support from runtime config.
- Metrics labels are bounded and privacy-safe.

## M6: Dedicated Benchmark Crate and Budgets

Goal: replace smoke-only benchmarking with release-grade scenarios.

### M6.1 Benchmark Crate

- [x] M6.1.1 Add `crates/kubio-bench`.
- [x] M6.1.2 Add local origin fixture.
- [x] M6.1.3 Add kubio process manager.
- [x] M6.1.4 Add h1 client.
- [x] M6.1.5 Add h2 client.
- [x] M6.1.6 Add h3 client behind feature.
- [x] M6.1.7 Emit JSON output.
- [x] M6.1.8 Emit budget pass/fail.

Acceptance:

- Benchmarks run from a clean checkout.
- Output includes safety and protocol counters.

### M6.2 Budgets

- [x] M6.2.1 Record v0.3.0 h1/h2 baseline.
- [x] M6.2.2 Record v0.3.1 h3 baseline.
- [x] M6.2.3 Define release budgets.
- [x] M6.2.4 Add release workflow artifact upload.
- [x] M6.2.5 Add release-note budget table.

Acceptance:

- Release candidate includes benchmark JSON and budget summary.

## M7: Release Hardening

Goal: ship v0.3.1 with clear experimental support boundaries.

- [x] M7.1 Existing workspace tests pass.
- [x] M7.2 Feature-enabled tests pass.
- [x] M7.3 Interoperability smoke passes or is skipped with documented environment reason.
- [x] M7.4 Privacy regression tests pass.
- [x] M7.5 Docker smoke covers HTTP/3 config.
- [x] M7.6 Release artifacts include HTTP/3 support level.
- [x] M7.7 Release notes state known limits and fallback behavior.

Acceptance:

- No safety regression from v0.3.0.
- HTTP/3 runtime support level is explicit.
- Release budgets are published.
