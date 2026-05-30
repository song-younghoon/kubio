# Testing and Release

Status: implemented gates passing for shipped slice; HTTP/3 runtime tests deferred
Target release: `v0.3.0`

## Goals

Testing must prove v0.3.0 improves performance and adds HTTP/2/HTTP/3 support without weakening v0.2.0 safety.

Implemented status:

- `cargo fmt --all --check` passes.
- `cargo test --workspace` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- CLI validation tests cover HTTP/2 h2c/TLS constraints and guarded HTTP/3 startup failure.
- Proxy integration coverage includes h2c prior-knowledge forwarding, safe reuse, protected requests, response hard-denies, revalidation, stale-if-error, cross-protocol cache-key equivalence, backpressure rejection behavior, HTTP/2 header-limit rejection, large protected response streaming behavior, oversized no-partial-store behavior, origin protocol fallback/fail-closed behavior, and retry fallback after HTTP/2 prior-knowledge connection failure.
- `examples/bench/local_smoke.sh` emits JSON with latency, cache, and protocol counters.
- `examples/bench/baseline_scenarios.sh` emits JSON for the HTTP/1.1 baseline scenario matrix and runs in CI/release workflow.

Deferred status:

- Full HTTP/3 QUIC runtime tests.
- Dedicated benchmark crate gates and recorded release budgets.
- External interoperability smoke tests such as `curl --http2`, `h2spec`, and `curl --http3`.

The release should be blocked by any regression that:

- Reuses protected traffic.
- Serves stale without permission.
- Miskeys responses across protocols.
- Buffers unbounded bodies.
- Exposes sensitive values.
- Claims HTTP/2 or HTTP/3 support without interoperability evidence.

## Required Existing Gates

Keep existing gates:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check
cargo audit
```

Add feature-specific gates:

```bash
cargo test --workspace --features http2
cargo test --workspace --features experimental-http3
```

If HTTP/3 uses unstable dependency cfg, put it in a separate CI job with explicit environment configuration.

## Unit Tests

Required new unit areas:

- Protocol config parsing and validation.
- TLS config validation.
- HTTP/2 h2c validation.
- HTTP/3 enabled/build-support validation.
- Origin protocol preference validation.
- Performance config bounds.
- Route hint index construction.
- Protocol enum serialization.
- Protocol metric label rendering.
- Backpressure limiter behavior.
- Observer sharding or queue overflow behavior.
- Store worker queue saturation behavior.
- Alt-Svc construction and skip reasons.

## Integration Tests

### Performance-Sensitive Proxy Paths

- Protected large response streams without full buffering.
- Known oversized response streams and is not stored.
- Storeable small response is still buffered, fingerprinted, and stored.
- Disk store write failure returns origin response.
- Async disk writes do not break purge behavior.
- Backpressure limit rejects new requests and does not affect already running requests.
- Panic switch disables reuse across all protocols.

### HTTP/2 Downstream

- TLS ALPN negotiates HTTP/2.
- TLS listener still supports HTTP/1.1 when enabled.
- h2c prior-knowledge works only when enabled.
- Multiplexed HTTP/2 safe GET requests can be observed, promoted, stored, and reused.
- HTTP/2 Authorization and Cookie requests are protected and never stored.
- HTTP/2 responses with `Set-Cookie`, `private`, `no-store`, unsupported `Vary`, and non-200 statuses are not stored.
- HTTP/2 revalidation sends validators and handles 304.
- HTTP/2 stale-if-error remains explicit and bounded.
- Malformed pseudo-header cases reject the stream without cache effects.

### HTTP/2 Upstream

- HTTPS origin negotiates HTTP/2 when available.
- Origin HTTP/2 required mode fails clearly when unavailable.
- Origin HTTP/2 preferred mode falls back when configured.
- Conditional revalidation works over upstream HTTP/2.
- Upstream protocol labels are bounded.

### HTTP/3 Downstream

Run when HTTP/3 feature is enabled:

- UDP listener starts.
- Safe GET over HTTP/3 can be observed, promoted, stored, and reused.
- Authorization and Cookie over HTTP/3 are protected and never stored.
- HTTP/3 request body on GET is protected.
- HTTP/3 malformed request rejects stream without cache effects.
- Alt-Svc is emitted only when configured.
- Disabling HTTP/3 removes the listener and Alt-Svc.

### HTTP/3 Upstream

Run in experimental CI:

- Origin HTTP/3 preferred mode attempts HTTP/3.
- Fallback to HTTP/2 or HTTP/1.1 works when configured.
- Required HTTP/3 mode fails clearly when unavailable.
- Protocol attempt and fallback are observable.

### Cross-Protocol Cache Semantics

- HTTP/1.1 and HTTP/2 clients share safe cached public responses when method/path/query/Vary keys match.
- HTTP/3 clients share the same cache semantics when enabled.
- Downstream protocol alone does not split cache keys.
- Upstream protocol alone does not split cache keys.
- Vary headers still split keys consistently.
- Sensitive requests are protected regardless of protocol.

## Interoperability Tests

Use available tools where practical:

- `curl --http2` for TLS HTTP/2.
- `curl --http2-prior-knowledge` for h2c.
- `h2spec` for HTTP/2 protocol compliance if available.
- `curl --http3` for HTTP/3 if the CI image supports it.
- `h3i` or a Rust h3 client for malformed HTTP/3 cases if available.

Interoperability tests can be optional locally but should run in at least one release CI workflow before tagging.

## Property Tests

Use `proptest` for:

- Protocol config combinations.
- Bounded metric label rendering.
- Alt-Svc value construction from port and max-age.
- Route hint index equivalence to v0.2.0 linear matching.
- Header normalization across HTTP/1.1, HTTP/2, and HTTP/3.
- Cache key equality across protocols when semantics match.
- Cache key inequality when Vary or query rules require separation.

## Performance Tests

Required benchmark scenarios:

- HTTP/1.1 pass-through safe GET.
- HTTP/1.1 protected streaming large body.
- HTTP/1.1 fresh memory hit.
- HTTP/1.1 fresh disk hit.
- HTTP/1.1 stale 304 revalidation.
- HTTP/2 multiplexed pass-through.
- HTTP/2 multiplexed fresh hit.
- HTTP/2 upstream origin requests.
- HTTP/3 downstream pass-through when enabled.
- HTTP/3 downstream fresh hit when enabled.
- Mixed 70% fresh, 10% revalidate, 10% protected, 5% stale-if-error, 5% miss.

Benchmark output must include:

- p50, p95, p99.
- Throughput.
- Origin request count.
- Reused response count.
- Protected request count.
- Revalidation and stale counts.
- Downstream and upstream protocol.
- Memory or store stats when available.

## Security and Privacy Tests

Required assertions:

- Authorization/Cookie/Set-Cookie values do not appear in metrics, dashboard APIs, logs, benchmark output, or disk files.
- Raw query values do not appear in protocol metrics.
- TLS private key material never appears in logs or dashboard output.
- QUIC connection IDs and tokens do not appear in metric labels.
- HTTP/2 connection-specific headers do not affect cache keys.
- HTTP/3 connection-specific headers do not affect cache keys.
- Alt-Svc is not advertised for unconfigured authorities.
- 0-RTT is disabled for HTTP/3.
- Protocol fallback does not retry unsafe request bodies unless the body is replayable and policy allows pass-through safely.

## Release Artifacts

v0.3.0 should publish:

- Source tag.
- Linux x86_64 binary.
- Docker image.
- Checksums.
- Release notes.
- Example HTTP/2 config.
- Example HTTP/3 experimental config.
- Benchmark output from release candidate.

Optional:

- macOS arm64 binary.
- Separate experimental HTTP/3 binary if dependency flags make the default binary undesirable.

## Smoke Tests

HTTP/1.1:

```bash
kubio serve --to http://localhost:3000
curl http://localhost:8080/api/products
curl http://127.0.0.1:9900/metrics
```

HTTP/2:

```bash
kubio serve --config examples/kubio-v0.3-http2.yml
curl --http2 -k https://localhost:8443/api/products
curl -k https://localhost:9900/api/overview
```

HTTP/3 experimental:

```bash
kubio serve --config examples/kubio-v0.3-http3.yml
curl --http3 -k https://localhost:8443/api/products
```

Expected:

- Proxy starts.
- Dashboard starts.
- Metrics expose protocol counters.
- Safe public traffic can reuse only after normal confidence gates.
- Protected traffic is never reused.
- HTTP/3 smoke is skipped clearly if the binary lacks HTTP/3 support.

## Release Exit Criteria

All must be true:

- Existing v0.1.0 and v0.2.0 safety tests pass.
- Benchmark harness exists and is documented.
- Performance budgets are recorded for the release candidate.
- Protected large responses stream without full buffering.
- Disk store work is off the async hot path.
- HTTP/2 downstream and upstream paths are tested.
- HTTP/3 downstream path is tested behind explicit feature/config.
- HTTP/3 upstream path is either tested as experimental or explicitly deferred.
- Protocol metrics and dashboard output use bounded labels.
- Sensitive values are absent from metrics, logs, dashboard APIs, disk files, and benchmark output.
- Documentation explains defaults, protocol support levels, fallback behavior, and migration from v0.2.0.
