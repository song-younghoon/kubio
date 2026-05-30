# PRD: kubio v0.3.0

Document version: v0.3 implemented scope
Product type: Open-source software
Primary implementation language: Rust
Release target: Protocol-aware performance
Core philosophy: **preserve safety, reduce overhead, support modern HTTP transports deliberately**

---

Implementation status: v0.3.0 landed the protocol/performance configuration surface, stable HTTP/2 downstream and upstream support, TLS ALPN, h2c prior knowledge, configurable HTTP/2 connection settings, request backpressure, origin pool tuning, guarded HTTP/3 config validation, route-hint fast paths, protocol/store/backpressure/fallback observability, and local benchmark/baseline smoke JSON output. The full HTTP/3 QUIC runtime, dedicated benchmark crate, and committed release budgets are deferred to a later release.

## 1. Product Summary

kubio v0.3.0 extends the v0.2.0 local-first API response reuse proxy with the runtime and protocol features needed for real production edges:

```text
performance measurement
hot-path overhead reduction
HTTP/2 downstream and upstream support
experimental HTTP/3 downstream and upstream support
protocol-aware observability
```

The core promise becomes:

> kubio can improve public API latency and origin load across modern HTTP transports while keeping its conservative reuse decisions protocol-independent.

## 2. Background and Problem

v0.2.0 can safely reuse, revalidate, persist, and explain public GET/HEAD responses, but it remains limited as a production proxy:

- The current benchmark story is a smoke script, not a release gate.
- The observer and store paths are simple and safe but can add lock contention under high request rates.
- Disk store operations use blocking filesystem calls from async store methods.
- Protected and unstoreable responses should stream with minimal buffering wherever possible.
- Before v0.3.0, the proxy used the simple Axum serve path and was effectively HTTP/1.1 for edge use.
- The upstream reqwest client is configured for HTTP/1.1 with rustls and streaming, but not explicitly for HTTP/2.
- HTTP/3 requires a different transport model over QUIC and should not be bolted directly into the HTTP/1.1 path without clear boundaries.

v0.3.0 should turn kubio from a safe local-first proxy into a measured, protocol-aware proxy without claiming CDN completeness.

## 3. Product Goals

kubio v0.3.0 should:

```text
1. Define repeatable performance scenarios and release budgets.
2. Reduce fresh-hit, pass-through, and protected-streaming hot-path overhead.
3. Avoid avoidable body buffering for traffic that cannot be stored or fingerprinted.
4. Move blocking disk work off the async hot path.
5. Reduce observer lock contention and keep event recording bounded.
6. Add configurable backpressure and origin connection pooling.
7. Accept HTTP/2 traffic from clients using TLS ALPN.
8. Support optional h2c prior-knowledge mode for local or service-mesh deployments.
9. Use HTTP/2 to capable origins with explicit fallback behavior.
10. Add an experimental HTTP/3 QUIC listener for client traffic.
11. Optionally try HTTP/3 to origins behind an explicit experimental feature.
12. Expose protocol, connection, stream, and cache behavior in metrics, dashboard APIs, CLI, and debug headers.
13. Preserve all v0.2.0 hard-deny, shadow-validation, revalidation, stale-if-error, query, and disk safety behavior.
```

## 4. User Experience Goals

### Performance

Users should be able to run:

```bash
bash examples/bench/local_smoke.sh
cargo run -p kubio-bench -- --scenario fresh-hit --protocol h1
cargo run -p kubio-bench -- --scenario fresh-hit --protocol h2
```

They should see:

```text
scenario=fresh-hit protocol=h2 requests=50000 p50=1.2ms p95=2.8ms origin_requests=0 reused=50000
```

The benchmark should report cache behavior and protocol behavior together so a fast but unsafe run cannot look successful.

### HTTP/2

Operators should be able to configure one HTTPS listener that supports HTTP/1.1 and HTTP/2:

```yaml
server:
  listen: "0.0.0.0:8443"
  tls:
    cert: "certs/kubio.pem"
    key: "certs/kubio-key.pem"
  protocols:
    http1: true
    http2: true
```

Origins can be configured separately:

```yaml
origin: "https://api.example.com"
origin_protocol:
  preferred: "auto" # auto | http1 | http2 | http3
  fallback: true
```

### HTTP/3

HTTP/3 should be explicit:

```yaml
server:
  http3:
    enabled: true
    listen: "0.0.0.0:8443"
    advertise: true
    max_concurrent_streams: 128
```

When enabled, kubio should:

- Bind UDP for QUIC.
- Use the same certificate identity as the HTTPS edge.
- Advertise `Alt-Svc` only when configured to do so.
- Serve the same safe cache decisions as HTTP/1.1 and HTTP/2.
- Fall back naturally when clients or networks do not support UDP/QUIC.

## 5. Non-Goals

kubio v0.3.0 will not provide:

```text
CDN-grade global edge routing
Distributed cache coherence
Kubernetes operator
Redis-compatible shared cache
Per-user private caching
Unsafe method reuse
CONNECT tunneling
WebSocket proxying
WebTransport
HTTP/3 datagram forwarding
Server push
Automatic TLS certificate issuance
Config reload without restart
```

## 6. Product Principles

### 6.1 Protocols Must Not Change Safety Semantics

HTTP versions carry the same HTTP semantics differently. kubio policy should not care whether a safe public GET arrived through HTTP/1.1, HTTP/2, or HTTP/3 once the request is normalized.

Protocol-specific parsing can reject malformed input, but it cannot relax reuse gates.

### 6.2 Measure Before Optimizing

v0.3.0 should land a benchmark harness before major hot-path changes. Every optimization should be compared against the v0.2.0 baseline:

- Pass-through safe GET.
- Fresh memory hit.
- Fresh disk hit.
- Stale revalidation with 304.
- Protected streaming response.
- Large unstoreable response.
- Dashboard metrics rendering under load.

### 6.3 HTTP/2 Is Stable Scope

HTTP/2 should be a normal supported feature in v0.3.0:

- Downstream TLS ALPN.
- Optional h2c prior knowledge.
- Upstream HTTP/2 to origins.
- Flow-control limits and stream concurrency limits.
- Metrics and tests.

### 6.4 HTTP/3 Is Guarded Scope

HTTP/3 should be usable and tested, but guarded:

- Disabled by default.
- Requires TLS certificate configuration.
- Uses bounded QUIC transport limits.
- Disables 0-RTT in v0.3.0.
- Treats upstream HTTP/3 as experimental because client library support is still less stable than HTTP/1.1 and HTTP/2.

### 6.5 Backpressure Is Better Than Unsafe Speed

If kubio is overloaded, it may shed new work with a clear status or let origin fallback happen where possible. It must not skip hard-deny checks, shadow eligibility checks, cache-key construction, or response safety checks to preserve throughput.

## 7. Success Metrics

Release success is measured by:

- CI has a benchmark job or documented benchmark workflow that records v0.3.0 budgets.
- Fresh memory hit p95 overhead is documented and does not regress from the v0.2.0 baseline after normalization.
- Protected large responses stream without full-body buffering.
- Disk writes do not block Tokio worker threads on the proxy hot path.
- HTTP/2 downstream integration tests prove multiplexed requests preserve cache safety.
- HTTP/2 upstream integration tests prove origin requests can negotiate or require HTTP/2.
- HTTP/3 downstream tests prove safe GET reuse and protected request pass-through over QUIC.
- HTTP/3 upstream tests run behind an explicit feature or CI job when the experimental dependency set is enabled.
- Metrics include bounded labels for downstream and upstream protocol.
- Existing v0.1.0 and v0.2.0 safety tests continue to pass.

## 8. Compatibility

Default behavior should remain close to v0.2.0:

- Default mode is still `watch`.
- Existing `origin: "http://..."` config remains valid.
- Existing HTTP/1.1 listener remains valid.
- Memory store remains default.
- HTTP/2 and HTTP/3 are opt-in unless TLS configuration and protocol defaults explicitly enable HTTP/2.
- HTTP/3 is disabled by default.
- Cache keys remain based on method, origin scheme, origin authority, path, normalized query, and supported Vary headers.
- Hard denies remain unchanged.

v0.3.0 config should reject inconsistent protocol settings before listeners bind.
