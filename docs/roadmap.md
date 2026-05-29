# Roadmap

v0.1.0:

- Local reverse proxy.
- Watch, shadow, and auto modes.
- In-memory cache.
- Local dashboard.
- Prometheus-style metrics.
- Conservative safety policy.
- Release notes draft: `docs/release-notes-v0.1.0.md`.

v0.2.0 design target:

- Conditional revalidation with ETag and Last-Modified.
- `Cache-Control: no-cache` as store-with-revalidation when safe.
- Bounded stale-if-error when origin headers or route policy explicitly allow it.
- Explicit route policy hints.
- Query parameter intelligence and opt-in query key hints.
- Process-local disk store.
- Dashboard, metrics, CLI, and docs for revalidation, stale, query, hint, and disk-store decisions.
- Design draft: `.design/v0.2.0`.

v0.3+ candidates:

- Redis-compatible shared store.
- Kubernetes deployment guide or operator.
- GraphQL opt-in mode.
- HTTP/2 support.
- Runtime config reload.
