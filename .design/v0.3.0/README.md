# kubio v0.3.0 Design Index

Status: implemented scope documented; HTTP/3 runtime deferred
Source: v0.2.0 implementation baseline and `docs/roadmap.md`
Target release: `v0.3.0`

This directory defines the v0.3.0 design for kubio. v0.2.0 made safe response reuse more practical with revalidation, bounded stale-if-error, route hints, query intelligence, and process-local disk persistence. The implemented v0.3.0 scope keeps that safety model while adding protocol and performance configuration, downstream and upstream HTTP/2 support, TLS ALPN, h2c prior knowledge, backpressure, protocol/store observability, local benchmark smoke output, and guarded HTTP/3 configuration.

The release theme is:

```text
Protocol-aware performance: lower hot-path overhead, stable HTTP/2, and guarded HTTP/3.
```

## Implementation Status

Implemented in the v0.3.0 codebase:

- Workspace version, config schema, docs, examples, and release notes for v0.3.0.
- HTTP/2 downstream support through TLS ALPN or explicit h2c prior knowledge.
- HTTP/2 upstream support through reqwest, including optional prior knowledge for trusted origins.
- TLS listener configuration with ALPN derived from enabled HTTP/1.1 and HTTP/2 protocols.
- Performance knobs for in-flight request limiting, bounded response buffering, unstoreable response streaming, and origin pool tuning.
- Protocol fallback events/metrics, live in-flight gauges, store operation metrics, observer event-drop metrics, and dashboard protocol summaries.
- Local benchmark smoke script that emits JSON latency, cache, and protocol counters.
- HTTP/3 configuration parsing and validation that fails startup clearly when QUIC runtime support is requested.

Deferred beyond the implemented v0.3.0 slice:

- Full HTTP/3 QUIC downstream and upstream runtime.
- Dedicated benchmark crate and release performance budgets.
- Deeper HTTP/2 per-connection flow-control tuning.

## Original Release Definition

The original release definition is retained below as the design target. The implemented v0.3.0 slice is recorded in the implementation status above; deferred items are future work.

kubio v0.3.0 is complete when a user can:

- Run repeatable local and CI performance benchmarks for pass-through, fresh hit, revalidation, stale-if-error, protected streaming, disk, HTTP/2, and HTTP/3 scenarios.
- Enable lower-overhead hot-path behavior without weakening the v0.2.0 safety model.
- Serve client traffic over HTTP/1.1 and HTTP/2 on the same TLS listener through ALPN.
- Optionally serve cleartext HTTP/2 prior-knowledge traffic for local and service-mesh deployments.
- Prefer or require HTTP/2 to capable origins, with HTTP/1.1 fallback unless disabled.
- Enable an experimental HTTP/3 downstream listener over QUIC with explicit TLS certificate configuration.
- Advertise HTTP/3 with `Alt-Svc` only when kubio is configured as an authoritative HTTPS edge for the request host.
- Optionally try HTTP/3 to origins behind an explicit experimental build/config flag, with fallback to HTTP/2 or HTTP/1.1.
- See protocol, connection, stream, cache, and latency behavior in metrics, dashboard APIs, CLI output, and debug headers.
- Upgrade from v0.2.0 without changing default safety, cache-key, or protected-traffic behavior.

## In Scope

- Benchmark harness and release performance budgets.
- Hot-path allocation, locking, buffering, body streaming, and disk I/O improvements.
- Configurable origin connection pooling, request concurrency, timeouts, and backpressure.
- HTTP/2 downstream support, including TLS ALPN and optional h2c prior knowledge.
- HTTP/2 upstream support to origins using ALPN or explicit prior knowledge.
- Experimental HTTP/3 downstream support using QUIC.
- Experimental HTTP/3 upstream support only when dependency and build stability are acceptable.
- Protocol-aware observability and operator-facing documentation.

## Out of Scope

- Redis or distributed cache coordination.
- Kubernetes operator.
- GraphQL POST response reuse.
- Authenticated per-user cache.
- Unsafe method reuse.
- WebSocket, CONNECT, WebTransport, or HTTP/3 datagram proxying.
- HTTP/2 server push or HTTP/3 server push.
- Fully RFC-complete HTTP cache semantics.
- Hosted control plane or required telemetry.

## Design Documents

- [PRD](PRD.md)
  - Product goals, user experience, release scope, non-goals, and success criteria.
- [Architecture Delta](architecture-delta.md)
  - Workspace changes, protocol abstraction, config model, proxy runtime changes, and failure behavior.
- [Performance Plan](performance-plan.md)
  - Benchmarks, budgets, hot-path optimizations, backpressure, and metrics.
- [HTTP/2 Support](http2-support.md)
  - Downstream and upstream HTTP/2 behavior, h2c, TLS ALPN, flow control, and cache semantics.
- [HTTP/3 Support](http3-support.md)
  - QUIC listener, Alt-Svc, upstream experiment, fallback, limits, and security constraints.
- [Observability and Dashboard](observability-dashboard.md)
  - Protocol metrics, dashboard/API updates, CLI output, and debug headers.
- [Testing and Release](testing-release.md)
  - Unit, integration, interoperability, performance, security, and release gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone work breakdown with acceptance checks.

## Cross-Cutting Constraints

- Safe default: unknown, risky, malformed, overloaded, or protocol-unsupported traffic goes to origin or receives an explicit gateway error. It must not be served from cache unless v0.2.0 reuse gates pass.
- Hard denies remain hard: Authorization, Cookie, unsafe methods, Set-Cookie, `private`, `no-store`, unsupported `Vary`, range requests, request bodies on GET/HEAD, and shadow mismatches cannot be relaxed by protocol support.
- Protocol support is transport, not policy: HTTP/1.1, HTTP/2, and HTTP/3 feed the same policy, cache-key, observer, and store decisions.
- HTTP/3 is opt-in for v0.3.0. It should be shippable as an experimental feature, not the default listener.
- Backpressure must be explicit. A saturated proxy may reject new work, but it must not skip safety checks to preserve throughput.
- Privacy defaults remain unchanged: do not persist or expose raw Authorization, Cookie, Set-Cookie, request bodies, raw query values, or validator values in default metrics.
- Labels must stay bounded. Protocol metrics can label `downstream_protocol` and `upstream_protocol`, but not arbitrary hosts, paths, query values, or header values.
- Fail open for cache/store/policy errors means pass through to origin when possible. Protocol negotiation errors may fall back to a lower configured protocol or return a bounded gateway error.

## Milestone Map

- M0: Design, dependency review, and schema preparation
- M1: Benchmark harness and baseline performance telemetry
- M2: Hot-path performance improvements
- M3: HTTP/2 downstream and upstream support
- M4: Experimental HTTP/3 support
- M5: Dashboard, metrics, CLI, docs, and examples
- M6: Release hardening and interoperability

Each milestone should preserve the v0.2.0 safety model. Partial v0.3.0 protocol work must fail closed for reuse and pass through to origin where possible.
