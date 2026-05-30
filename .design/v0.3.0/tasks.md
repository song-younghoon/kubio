# v0.3.0 Implementation Tasks

Status: local v0.3.0 implementation complete; HTTP/3 runtime and release budgets deferred
Target release: `v0.3.0`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete

## Current Implementation Snapshot

v0.2.0 baseline exists:

- HTTP/1.1 reverse proxy.
- Watch, shadow, and auto modes.
- Conservative hard-deny policy.
- Shadow validation and auto reuse for verified public GET/HEAD responses.
- Conditional revalidation with `ETag` and `Last-Modified`.
- `Cache-Control: no-cache` store-with-revalidation when safe.
- Bounded stale-if-error.
- Route hints and query key hints.
- In-memory and process-local disk stores.
- Local dashboard, JSON APIs, metrics, admin purge, doctor, and docs.

v0.3.0 adds the implemented protocol/performance configuration surface, HTTP/2 support, guarded HTTP/3 configuration, local benchmark smoke JSON output, protocol/backpressure/fallback/store observability, and release workflow artifacts. Full HTTP/3 QUIC runtime, a dedicated benchmark crate, committed benchmark budgets, observer sharding, and deeper HTTP/2 per-connection tuning remain deferred.

## M0: Design, Dependency Review, and Schema Preparation

Goal: prepare config and dependency choices without changing runtime behavior.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.3.0 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add architecture delta.
- [x] M0.1.4 Add performance plan.
- [x] M0.1.5 Add HTTP/2 and HTTP/3 design docs.
- [x] M0.1.6 Add testing and task breakdown.

Acceptance:

- Design is linked from roadmap and README.
- Scope clearly separates stable HTTP/2 from experimental HTTP/3.

### M0.2 Dependency Review

- [x] M0.2.1 Review Hyper/hyper-util server configuration needs.
- [x] M0.2.2 Review rustls/tokio-rustls TLS acceptor integration.
- [x] M0.2.3 Review reqwest HTTP/2 feature requirements.
- [x] M0.2.4 Review h3, h3-quinn, and Quinn for downstream HTTP/3.
- [x] M0.2.5 Review reqwest HTTP/3 instability and build requirements.
- [x] M0.2.6 Decide whether to add `kubio-transport`.

Status note: v0.3.0 uses Axum/Hyper HTTP/2, reqwest HTTP/2, and tokio-rustls. HTTP/3 dependencies and a dedicated transport crate are deferred until the QUIC runtime is implemented.

Acceptance:

- Dependency decisions are documented.
- HTTP/3 support level is explicit.
- Supply-chain CI remains passing.

### M0.3 Config Types

- [x] M0.3.1 Add protocol enums.
- [x] M0.3.2 Add TLS config.
- [x] M0.3.3 Add server protocol config.
- [x] M0.3.4 Add HTTP/2 config.
- [x] M0.3.5 Add HTTP/3 config.
- [x] M0.3.6 Add origin protocol config.
- [x] M0.3.7 Add performance config.
- [x] M0.3.8 Validate incompatible protocol config.

Acceptance:

- Existing v0.2.0 config remains valid.
- Invalid protocol config fails before listeners bind.
- HTTP/3 config fails clearly when unsupported by the build.

## M1: Benchmark Harness and Baseline

Goal: measure v0.2.0-equivalent behavior before optimizing.

Status note: `examples/bench/local_smoke.sh` is the v0.3.0 equivalent reproducible harness and emits JSON with latency, cache, and protocol counters. Full baseline scenario coverage and committed release budgets are deferred.

### M1.1 Benchmark Harness

- [x] M1.1.1 Add `kubio-bench` crate or equivalent reproducible harness.
- [x] M1.1.2 Start local origin fixtures from benchmark runs.
- [x] M1.1.3 Start kubio with scenario-specific config.
- [x] M1.1.4 Warm routes to auto eligibility.
- [x] M1.1.5 Emit JSON benchmark output.
- [x] M1.1.6 Record cache and protocol counters with latency.

Acceptance:

- Benchmarks run from a clean checkout.
- Output includes safety counters, not only latency.

### M1.2 Baseline Scenarios

- [ ] M1.2.1 HTTP/1.1 pass-through safe GET.
- [ ] M1.2.2 HTTP/1.1 protected request.
- [ ] M1.2.3 HTTP/1.1 fresh memory hit.
- [ ] M1.2.4 HTTP/1.1 fresh disk hit.
- [ ] M1.2.5 HTTP/1.1 304 revalidation.
- [ ] M1.2.6 HTTP/1.1 stale-if-error.
- [ ] M1.2.7 Large unstoreable response.
- [ ] M1.2.8 Metrics render under load.

Acceptance:

- Baseline numbers are committed or attached to release candidate notes.
- Performance budgets are defined from baseline variance.

## M2: Hot-Path Performance Improvements

Goal: reduce overhead while preserving safety decisions.

### M2.1 Route and Config Fast Paths

- [ ] M2.1.1 Build route hint index at config load.
- [ ] M2.1.2 Precompute route hint vary names.
- [ ] M2.1.3 Avoid repeated path/template normalization where possible.
- [ ] M2.1.4 Add tests proving index behavior matches existing matching.

Acceptance:

- Duplicate hints still fail validation.
- Route matching remains deterministic.

### M2.2 Streaming and Buffering

- [x] M2.2.1 Decide unstoreable response streaming before full body buffering.
- [x] M2.2.2 Add bounded buffering config.
- [x] M2.2.3 Preserve safety observations for streamed responses.
- [x] M2.2.4 Add large protected response test.
- [x] M2.2.5 Add oversized storeable response test.

Acceptance:

- Protected large responses are not fully buffered.
- No partial response is stored.

### M2.3 Store Hot Path

- [x] M2.3.1 Move disk reads/writes off Tokio worker threads.
- [x] M2.3.2 Add bounded store worker or `spawn_blocking`.
- [x] M2.3.3 Add store operation latency metrics.
- [x] M2.3.4 Add store saturation event.
- [x] M2.3.5 Preserve purge correctness.

Status note: disk store async blocking behavior predates this v0.3.0 patch but is part of the current shipped baseline.

Acceptance:

- Disk store tests still pass.
- Disk write failures return origin response.

### M2.4 Observer Hot Path

- [ ] M2.4.1 Replace or shard single observer lock.
- [x] M2.4.2 Keep shadow mismatch demotion deterministic.
- [x] M2.4.3 Add bounded event overflow behavior.
- [x] M2.4.4 Add observer dropped event metric.
- [ ] M2.4.5 Ensure dashboard snapshots do not block proxy updates for long periods.

Acceptance:

- Existing promotion and shadow mismatch tests pass.
- Load benchmark shows reduced observer contention.

### M2.5 Backpressure and Pooling

- [x] M2.5.1 Add global in-flight request limiter.
- [x] M2.5.2 Return bounded 503 on limiter saturation.
- [x] M2.5.3 Add origin pool config.
- [x] M2.5.4 Add timeout config validation.
- [x] M2.5.5 Add metrics for in-flight and rejections.

Acceptance:

- Backpressure does not relax cache safety.
- Origin pool behavior is observable.

## M3: HTTP/2 Downstream and Upstream

Goal: support stable HTTP/2 traffic.

### M3.1 Downstream HTTP/2

- [x] M3.1.1 Add TLS acceptor with ALPN.
- [ ] M3.1.2 Replace simple serve path where configurable HTTP/2 is required.
- [x] M3.1.3 Support HTTP/1.1 and HTTP/2 on same TLS listener.
- [x] M3.1.4 Add explicit h2c prior-knowledge mode.
- [x] M3.1.5 Normalize HTTP/2 pseudo headers.
- [~] M3.1.6 Enforce header and stream limits. HTTP/2 header-list limits and global stream/request backpressure are enforced; deeper per-connection stream tuning remains deferred.

Status note: v0.3.0 relies on Axum/Hyper for HTTP/2 handling. Config fields for HTTP/2 limits are parsed and validated, but deeper per-connection tuning is deferred.

Acceptance:

- HTTP/2 client can call kubio over TLS.
- h2c works only when enabled.
- HTTP/1.1 quick start still works.

### M3.2 Upstream HTTP/2

- [x] M3.2.1 Enable reqwest HTTP/2 feature/config.
- [x] M3.2.2 Add origin protocol preference config.
- [x] M3.2.3 Support required HTTP/2 mode.
- [~] M3.2.4 Support fallback to HTTP/1.1. Negotiated fallback is recorded; retry fallback after h2 prior-knowledge connection failure remains deferred.
- [x] M3.2.5 Record upstream protocol when known.

Acceptance:

- HTTP/2 origin tests pass.
- Fallback behavior is deterministic and visible.

### M3.3 HTTP/2 Safety Tests

- [x] M3.3.1 Safe GET reuse over HTTP/2.
- [x] M3.3.2 Authorization/Cookie protection over HTTP/2.
- [x] M3.3.3 Set-Cookie/private/no-store protection over HTTP/2.
- [x] M3.3.4 Revalidation over HTTP/2.
- [x] M3.3.5 Stale-if-error over HTTP/2.
- [x] M3.3.6 Cross-protocol cache key equivalence.

Status note: HTTP/2 safety coverage currently uses explicit h2c prior knowledge in the workspace integration suite. External TLS ALPN interoperability smoke remains a release-hardening task.

Acceptance:

- Protocol version alone does not split safe cache keys.
- Hard denies remain hard.

## M4: Experimental HTTP/3

Goal: add guarded HTTP/3 support.

### M4.1 Downstream HTTP/3

Status note: full QUIC runtime is deferred. v0.3.0 ships guarded config validation rather than a downstream HTTP/3 listener.

- [ ] M4.1.1 Add HTTP/3 Cargo feature.
- [ ] M4.1.2 Add QUIC endpoint setup.
- [ ] M4.1.3 Add h3 request adapter.
- [ ] M4.1.4 Add h3 response writer.
- [ ] M4.1.5 Disable 0-RTT.
- [ ] M4.1.6 Enforce stream/header/QPACK limits.

Acceptance:

- HTTP/3 listener starts only when enabled.
- Safe GET can flow through policy/cache handler.

### M4.2 Alt-Svc

Status note: config fields exist, but Alt-Svc emission is deferred with the HTTP/3 runtime.

- [ ] M4.2.1 Add Alt-Svc config.
- [ ] M4.2.2 Emit Alt-Svc only for valid configured authorities.
- [ ] M4.2.3 Add skip reasons.
- [ ] M4.2.4 Add metrics/events.

Acceptance:

- Alt-Svc is opt-in and bounded.
- Alt-Svc is not emitted for unconfigured authorities.

### M4.3 Upstream HTTP/3 Experiment

- [ ] M4.3.1 Decide reqwest unstable feature or dedicated h3 client.
- [x] M4.3.2 Add experimental build/config gate.
- [ ] M4.3.3 Implement preferred HTTP/3 origin path.
- [ ] M4.3.4 Implement fallback.
- [ ] M4.3.5 Add experimental CI tests.

Acceptance:

- HTTP/3 upstream can be disabled cleanly.
- Failure behavior is explicit.

### M4.4 HTTP/3 Safety Tests

Status note: runtime tests are deferred. CLI validation covers guarded HTTP/3 startup failure.

- [ ] M4.4.1 Safe GET reuse over HTTP/3.
- [ ] M4.4.2 Authorization/Cookie protection over HTTP/3.
- [ ] M4.4.3 Malformed request rejection.
- [ ] M4.4.4 Protocol metrics.
- [ ] M4.4.5 Cross-protocol cache key equivalence.

Acceptance:

- HTTP/3 does not change reuse safety.
- HTTP/3 metrics are privacy-safe.

## M5: Observability, CLI, Docs, and Examples

Goal: expose v0.3.0 behavior clearly.

### M5.1 Metrics and Events

- [x] M5.1.1 Add downstream protocol metrics.
- [x] M5.1.2 Add upstream protocol metrics.
- [x] M5.1.3 Add protocol fallback metrics.
- [x] M5.1.4 Add backpressure metrics.
- [x] M5.1.5 Add bounded protocol events.

Acceptance:

- Metrics labels are bounded.
- Sensitive values are absent.

### M5.2 Dashboard APIs and UI

- [x] M5.2.1 Extend overview API.
- [x] M5.2.2 Extend route detail API.
- [x] M5.2.3 Add store operation stats.
- [x] M5.2.4 Show protocol mix.
- [x] M5.2.5 Show fallback/backpressure warnings.

Acceptance:

- User can inspect protocol and performance behavior.

### M5.3 CLI and Docs

- [x] M5.3.1 Update `kubio doctor`.
- [x] M5.3.2 Update `kubio routes`.
- [x] M5.3.3 Update `kubio explain`.
- [x] M5.3.4 Add HTTP/2 example config.
- [x] M5.3.5 Add HTTP/3 guarded example config.
- [x] M5.3.6 Update README and configuration docs.
- [x] M5.3.7 Draft release notes.

Acceptance:

- Docs state stable versus experimental support.
- CLI output remains compact and redacted.

## M6: Release Hardening

Goal: ship v0.3.0 with safety, performance, and interoperability confidence.

### M6.1 Test Gates

- [x] M6.1.1 Existing workspace tests pass.
- [x] M6.1.2 HTTP/2 feature tests pass.
- [x] M6.1.3 HTTP/3 experimental tests pass or are explicitly deferred.
- [x] M6.1.4 Interoperability smoke tests run.
- [x] M6.1.5 Privacy regression tests pass.
- [x] M6.1.6 Benchmark harness runs in CI or release workflow.

Status note: verified with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `git diff --check`, `REQUESTS=10 MODE=auto bash examples/bench/local_smoke.sh`, `KUBIO_BIN=target/release/kubio bash examples/bench/release_smoke.sh`, `sha256sum target/release/kubio`, and `KUBIO_IMAGE=kubio:ci bash examples/bench/docker_smoke.sh`.

Acceptance:

- No safety regression from v0.2.0.
- Release notes include benchmark results.

### M6.2 Release Artifacts

- [x] M6.2.1 Linux binary.
- [x] M6.2.2 Docker image.
- [x] M6.2.3 Checksums.
- [x] M6.2.4 HTTP/2 smoke config.
- [x] M6.2.5 HTTP/3 experimental smoke config.
- [x] M6.2.6 Release notes.

Acceptance:

- Artifact smoke tests pass.
- HTTP/3 support level is clear in release notes.
