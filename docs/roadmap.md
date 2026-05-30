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

- Workspace version bump to `0.3.0`.
- Performance config for response buffering and origin connection pooling.
- Existing disk store operations run off Tokio worker threads.
- HTTP/2 downstream support via explicit h2c prior knowledge or TLS ALPN.
- HTTP/2 upstream support with origin protocol preference and optional prior knowledge.
- Guarded HTTP/3 config validation that fails clearly because the QUIC runtime is not in the default build.
- Protocol fallback metrics/events, live in-flight gauges, store operation metrics, and dashboard protocol summaries.
- Local benchmark smoke output with JSON latency, cache, and protocol counters.
- Protocol and performance config docs, examples, and release notes.
- Design status: `.design/v0.3.0` updated to reflect the implemented slice and deferred runtime work.
- Release notes: `docs/release-notes-v0.3.0.md`.

v0.4+ candidates:

- Redis-compatible shared store.
- Kubernetes deployment guide or operator.
- GraphQL opt-in mode.
- HTTP/3 QUIC runtime.
- Dedicated benchmark crate and release budgets.
- Observer sharding and deeper HTTP/2 flow-control tuning.
- Runtime config reload.
