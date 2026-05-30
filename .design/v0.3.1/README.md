# kubio v0.3.1 Design Index

Status: implementation in progress
Source: v0.3.0 deferred HTTP/3 runtime and benchmark work
Target release: `v0.3.1`

This directory defines the v0.3.1 design for turning v0.3.0's guarded HTTP/3 configuration into an actual experimental runtime. v0.3.1 may add new dependencies and make larger structure changes. The release should still preserve kubio's core safety rule: protocol support is transport, not policy.

The release theme is:

```text
Experimental HTTP/3 runtime with measured safety.
```

## Baseline

v0.3.0 already ships:

- HTTP/1.1 reverse proxy behavior.
- Stable downstream and upstream HTTP/2.
- TLS ALPN for HTTP/1.1 and HTTP/2.
- h2c prior knowledge.
- Protocol config, HTTP/3 config fields, and guarded HTTP/3 startup failures.
- Protocol/backpressure/fallback/store observability.
- Local benchmark and HTTP/1.1 baseline scenario smoke output.

v0.3.1 should add:

- A `kubio-transport` crate or equivalent transport boundary.
- Feature-gated HTTP/3 runtime dependencies.
- Downstream HTTP/3 listener over QUIC.
- HTTP/3 request adapter and response writer into the existing policy/cache handler.
- Safe, opt-in `Alt-Svc` emission.
- Upstream HTTP/3 experiment with deterministic fallback.
- HTTP/3 safety tests, interoperability smoke, and bounded metrics.
- A dedicated `kubio-bench` crate and release performance budgets.

## Dependency Direction

As of May 30, 2026, current public crate docs still make HTTP/3 a more experimental dependency choice than HTTP/1.1 and HTTP/2:

- `h3` provides async HTTP/3 client and server APIs over a generic QUIC transport, but its docs describe the crate as experimental.
- `h3-quinn` integrates `h3` with Quinn.
- `quinn` provides a pure-Rust async QUIC transport and rustls-backed TLS integration.
- `reqwest` exposes HTTP/3 only as an unstable feature that requires `--cfg reqwest_unstable`.

Design decision:

- Use `h3` + `h3-quinn` + `quinn` for downstream HTTP/3.
- Introduce an explicit `experimental-http3` Cargo feature across `kubio-cli`, `kubio-proxy`, and `kubio-transport`.
- Prefer a direct `h3`/Quinn upstream client for v0.3.1 if the needed fallback, protocol-attempt, certificate, and timeout controls are tractable.
- Allow reqwest HTTP/3 only behind a separate implementation switch if it proves safer to ship, because it requires unstable cfg and may change in patch releases.

## Scope

In scope:

- QUIC UDP listener on the configured HTTP/3 address.
- TLS 1.3 certificate reuse with ALPN `h3`.
- HTTP/3 request normalization into the existing handler.
- HTTP/3 response writing, including streaming response bodies.
- Disabled 0-RTT.
- Bounded stream, header, QPACK, idle, and body buffering limits.
- Opt-in `Alt-Svc` advertisement for configured authorities only.
- Upstream HTTP/3 for HTTPS origins behind explicit build and config gates.
- Fallback from upstream HTTP/3 to HTTP/2 or HTTP/1.1 for replayable requests only.
- HTTP/3 protocol metrics, events, dashboard fields, and debug headers with bounded labels.
- Dedicated benchmark crate and release budgets covering HTTP/1.1, HTTP/2, and HTTP/3.

Out of scope:

- Default-on HTTP/3.
- QUIC 0-RTT.
- HTTP/3 datagrams.
- WebTransport.
- CONNECT-UDP.
- WebSocket or CONNECT proxying.
- Server push.
- Multipath QUIC.
- CDN-grade edge routing or global cache coordination.
- Per-user private caching or unsafe method reuse.

## Documents

- [PRD](PRD.md)
  - Product goals, user experience, non-goals, and success metrics.
- [Architecture Delta](architecture-delta.md)
  - Workspace changes, crate boundaries, transport APIs, and failure model.
- [Dependency Review](dependency-review.md)
  - Selected HTTP/3 crates, feature strategy, and supply-chain status.
- [HTTP/3 Runtime](http3-runtime.md)
  - Downstream QUIC listener, adapters, Alt-Svc, upstream HTTP/3, and security controls.
- [Performance and Benchmarks](performance-and-benchmarks.md)
  - Dedicated benchmark crate, scenarios, and v0.3.1 release budgets.
- [Observability](observability.md)
  - Metrics, events, dashboard/API fields, CLI, and debug headers.
- [Testing and Release](testing-release.md)
  - Unit, integration, interoperability, security, feature, and release gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone task breakdown.

## Cross-Cutting Constraints

- Safety gates do not change based on protocol.
- Cache keys do not include downstream or upstream protocol unless a future design explicitly requires it.
- Authorization, Cookie, unsafe methods, Set-Cookie, `private`, `no-store`, unsupported `Vary`, Range, GET/HEAD bodies, and shadow mismatches remain hard deny or protected conditions.
- Malformed HTTP/3 requests must be rejected before entering the cache path.
- QUIC errors must not create cache entries, poison observer state, or expose sensitive details.
- HTTP/3 config without build support must fail clearly.
- A saturated HTTP/3 runtime may shed work, but it must not skip policy checks to preserve throughput.
- Metrics labels must remain bounded and cannot include arbitrary authority, path, query, header, certificate, QUIC connection ID, token, or peer address values.

## Milestone Map

- M0: Dependency and architecture spike.
- M1: Transport boundary and feature gates.
- M2: Downstream HTTP/3 runtime.
- M3: Alt-Svc and authority validation.
- M4: Upstream HTTP/3 experiment and fallback.
- M5: Observability, CLI, docs, and examples.
- M6: Dedicated benchmark crate and release budgets.
- M7: Release hardening and interoperability.
