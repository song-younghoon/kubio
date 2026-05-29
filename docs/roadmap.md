# Roadmap

v0.1.0:

- Local reverse proxy.
- Watch, shadow, and auto modes.
- In-memory cache.
- Local dashboard.
- Prometheus-style metrics.
- Conservative safety policy.
- Release notes draft: `docs/release-notes-v0.1.0.md`.

v0.2.0:

- Conditional revalidation with ETag and Last-Modified.
- `Cache-Control: no-cache` as store-with-revalidation when safe.
- Bounded stale-if-error when origin headers or route policy explicitly allow it.
- Explicit route policy hints.
- Query parameter intelligence and opt-in query key hints.
- Process-local disk store.
- Dashboard, metrics, CLI, and docs for revalidation, stale, query, hint, and disk-store decisions.
- Release notes draft: `docs/release-notes-v0.2.0.md`.

v0.3.0:

- Performance benchmark harness and release budgets.
- Hot-path performance improvements for route matching, buffering, observer contention, disk I/O, backpressure, and origin pooling.
- HTTP/2 downstream support with TLS ALPN and optional h2c prior knowledge.
- HTTP/2 upstream support with origin protocol preference and fallback.
- Experimental HTTP/3 downstream support over QUIC.
- Experimental HTTP/3 upstream support behind explicit build/config flags.
- Protocol-aware metrics, dashboard/API, CLI, docs, examples, and release notes.
- Design draft: `.design/v0.3.0`.

v0.4+ candidates:

- Redis-compatible shared store.
- Kubernetes deployment guide or operator.
- GraphQL opt-in mode.
- Runtime config reload.
