# kubio v0.3.0 Release Notes

Status: implemented scope; local release packaging verified, publishing pending

v0.3.0 adds the first protocol-aware runtime work for kubio while preserving the v0.2.0 safety model.

## Added

- Workspace version bumped to `0.3.0`.
- Downstream HTTP/2 support through a configurable Hyper connection builder.
- Explicit h2c prior-knowledge config for local and service-mesh cleartext HTTP/2 deployments.
- TLS certificate config for the proxy listener with ALPN values derived from enabled HTTP/1.1 and HTTP/2 protocols.
- Upstream HTTP/2 support through reqwest's HTTP/2 feature.
- Origin protocol config for `auto`, `http1`, `http2`, and guarded `http3` preferences.
- Optional HTTP/2 prior knowledge for trusted origins.
- Performance config for max buffered response size, streaming unstoreable bodies, observer contention planning, async disk write policy, and origin connection pool tuning.
- Route-hint derived state for indexed hint lookup and precomputed vary names.
- Bounded protocol counters for downstream and upstream traffic, protocol fallback counters/events, and backpressure rejection counters/events.
- Live in-flight request gauges, bounded observer event-drop counters, and store operation counters/latency metrics.
- HTTP/2 header-list limit enforcement at the proxy request boundary.
- Large protected and oversized storeable response regression tests to keep streaming and no-partial-store behavior intact.
- Local benchmark smoke output with JSON latency, cache, and protocol counters, now wired into CI.
- HTTP/1.1 baseline scenario smoke for pass-through, protected, memory/disk hit, 304 revalidation, stale-if-error, large unstoreable response, and metrics-under-load paths.
- Guarded HTTP/3 config validation. HTTP/3 settings are parsed, but enabling downstream or upstream HTTP/3 fails clearly because the QUIC runtime is not included in the default build.
- v0.3.0 HTTP/2 and guarded HTTP/3 example configs.

## Changed

- The origin client now uses configured connection pool limits and idle timeout.
- Downstream HTTP/2 now applies configured stream, window, keepalive, and header-list settings at the Hyper connection builder.
- Observer snapshots clone state under a read lock and perform sorting/aggregation outside the proxy update path.
- Known oversized responses also consider `performance.max_buffered_response_size` when deciding whether to stream instead of buffer for storage.
- Startup output now reports enabled downstream protocols and origin protocol preference.
- Configuration docs now describe v0.3.0 protocol and performance settings.

## Safety

- HTTP/2 uses the same request policy, response policy, cache-key, revalidation, stale-if-error, route hint, and query hint behavior as HTTP/1.1.
- Protected traffic remains protected across HTTP/1.1 and HTTP/2.
- HTTP/3 is not silently ignored: enabling it in this build fails startup with a clear operator-facing error.

## Known Limits

- The default v0.3.0 build does not include the HTTP/3 QUIC runtime.
- Full benchmark budgets remain a follow-up until baseline variance is recorded.
- The release workflow builds Linux artifacts and checksums; publishing those artifacts still happens outside the local test run.

## Local Verification

- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `REQUESTS=10 MODE=auto bash examples/bench/local_smoke.sh`
- `bash examples/bench/baseline_scenarios.sh`
- `KUBIO_BIN=target/release/kubio bash examples/bench/release_smoke.sh`
- `docker build -t kubio:ci .`
- `KUBIO_IMAGE=kubio:ci bash examples/bench/docker_smoke.sh`
