# System Architecture

Status: draft
Target release: `v0.1.0`

## Goals

The architecture must support a single self-contained Rust binary that can run as a reverse proxy, observe traffic, make conservative policy decisions, serve a local dashboard, and expose metrics without a required external service.

The most important architectural property is fail-open behavior: if any optional subsystem fails, proxy traffic should continue to reach the origin unless the origin itself is unavailable.

## Workspace Layout

Use a Cargo workspace with focused crates:

```text
kubio/
  Cargo.toml
  crates/
    kubio-cli/
    kubio-core/
    kubio-proxy/
    kubio-policy/
    kubio-observe/
    kubio-store/
    kubio-dashboard/
    kubio-telemetry/
  docs/
  examples/
```

### `kubio-cli`

Responsibilities:

- Parse CLI subcommands and flags.
- Load and merge configuration.
- Validate startup configuration before binding sockets.
- Own process lifecycle and graceful shutdown.
- Start proxy, dashboard, metrics, telemetry, and shared state.
- Print first-run startup output.

Primary dependencies:

- `clap` for CLI parsing.
- `serde`, `serde_yaml`, `humantime-serde` or equivalent for config.
- `tokio` for runtime and signal handling.

### `kubio-core`

Responsibilities:

- Shared types and lightweight utilities.
- HTTP-neutral identifiers such as `RouteId`, `CacheKeyHash`, `Decision`, `DecisionReason`, `RouteState`, `FreshnessProfile`.
- Time, byte-size, status-class, and redaction helpers.
- Stable serialization types used by dashboard and CLI.

This crate should avoid depending on the proxy or dashboard layers.

### `kubio-proxy`

Responsibilities:

- Accept inbound HTTP/1.1 requests.
- Forward requests to the configured origin.
- Strip or handle hop-by-hop headers.
- Stream request and response bodies where possible.
- Bridge response body observation, fingerprinting, and optional cache storage.
- Apply policy decisions and fail open on internal errors.

Primary dependencies:

- `hyper` or `axum`/`tower` on top of `hyper`.
- `http`, `http-body-util`, `bytes`.
- `tokio`.

Recommendation: use `axum` for dashboard/admin APIs and direct `hyper` or `tower` services for proxy hot-path control. If one framework is chosen for both, the proxy path must still preserve streaming and avoid dashboard coupling.

### `kubio-policy`

Responsibilities:

- Extract request and response safety signals.
- Classify protected/bypass/store/reuse decisions.
- Parse safety-critical response headers.
- Score route cacheability.
- Select freshness profile TTLs.
- Build explanations for route and request decisions.
- Own promotion/demotion rules from observed state to route state.

This crate should be deterministic and heavily unit-tested.

### `kubio-observe`

Responsibilities:

- Maintain in-memory route statistics.
- Maintain bounded cache-key observation state.
- Track fingerprint stability.
- Track shadow matches and mismatches.
- Emit recent events.
- Serve read-only snapshots to dashboard and CLI.

Observation state must use hashes and flags, not raw sensitive values.

### `kubio-store`

Responsibilities:

- Define the `CacheStore` trait.
- Implement process-local memory storage.
- Support TTL expiration.
- Enforce max object size and max total size.
- Track entry count, cache bytes, and evictions.
- Support purge by all, route, or key hash.

Memory store should be designed as an implementation detail, not leaked through policy logic.

### `kubio-dashboard`

Responsibilities:

- Serve local dashboard UI and JSON APIs.
- Expose overview, routes, route detail, recent events, config view, and optional purge endpoint.
- Bind to `127.0.0.1:9900` by default.
- Refuse or warn on public binding unless explicitly configured.

Dashboard failure must not stop the proxy listener.

### `kubio-telemetry`

Responsibilities:

- Structured logs with sensitive header redaction.
- Prometheus-compatible metrics.
- Trace spans around proxy, origin, policy, store, and observation work.
- Metrics labels with bounded cardinality.

The telemetry crate should expose stable recording APIs so core code does not depend directly on a metrics backend.

## Runtime Topology

```text
                 +------------------+
client traffic ->| proxy listener   |----+
                 +------------------+    |
                                         v
                 +------------------+  +----------------+
dashboard -----> | dashboard server |  | origin client  | -> origin API
                 +------------------+  +----------------+
                         |                  ^
                         v                  |
                 +------------------+       |
metrics scrape ->| metrics endpoint |       |
                 +------------------+       |
                         |                  |
                         v                  |
                 +--------------------------------------+
                 | shared app state                     |
                 | config, policy, observe, store, etc. |
                 +--------------------------------------+
```

Use a single Tokio runtime. Start proxy, dashboard, and metrics as independent tasks supervised by `kubio-cli`.

Shared state should be cheap to clone:

```rust
pub struct AppState {
    pub config: Arc<EffectiveConfig>,
    pub policy: Arc<PolicyEngine>,
    pub observer: Arc<Observer>,
    pub store: Arc<dyn CacheStore>,
    pub telemetry: Telemetry,
    pub panic_switch: Arc<PanicSwitch>,
}
```

## Request Path Ownership

The proxy hot path owns request execution. It calls other crates through narrow APIs:

```text
proxy
  -> core route/key helpers
  -> policy request precheck
  -> store lookup when eligible
  -> origin client when needed
  -> policy response decision
  -> observe record
  -> telemetry record
```

Dashboard and CLI must read snapshots from observation state rather than locking hot-path internals for long periods.

## State Model

### Durable State

v0.1.0 has no required durable runtime state. All observations and cache entries are process-local memory.

### In-Memory State

- Effective configuration.
- Route statistics.
- Bounded per-key fingerprint history.
- Recent event ring buffer.
- Memory cache entries.
- Metrics counters/histograms/gauges.

All in-memory maps must have bounds or eviction strategy before release.

## Concurrency Model

Use lock granularity that protects the proxy path:

- Prefer sharded maps such as `DashMap` or carefully scoped `RwLock<HashMap<...>>`.
- Route snapshots should copy compact structs out of live state.
- Cache store operations should avoid holding global locks while cloning large bodies.
- Event recording should use a bounded ring buffer.
- Dashboard reads should tolerate slightly stale data.

## Configuration Model

Configuration sources, lowest to highest precedence:

1. Safe built-in defaults.
2. Optional YAML config file.
3. CLI flags.

Configuration validation happens before listeners start:

- Origin URL must be absolute HTTP/HTTPS URL.
- Listen/dashboard addresses must parse.
- Mode must be one of `watch`, `shadow`, `auto`.
- Size and TTL values must be valid.
- Public dashboard binding requires explicit opt-in.
- If dashboard public binding has admin routes enabled, require an admin token.

Effective configuration should be serializable for the dashboard config page, with secrets redacted.

## Failure Model

The proxy must treat internal subsystem errors as reasons to pass through to origin.

| Failure | Required behavior |
| --- | --- |
| Policy error | Pass through to origin, record `PolicyError` if possible |
| Store get error | Pass through to origin, record `StoreError` |
| Store put error | Return origin response, record store error |
| Observation error | Continue request, log redacted error |
| Fingerprint error | Mark ineligible for auto reuse |
| Dashboard bind failure | Proxy still starts unless dashboard was explicitly required |
| Metrics failure | Proxy continues |
| Panic switch active | No cached responses served |
| Origin failure | Return appropriate `502` or `504` |

## Dependency Principles

- Keep policy deterministic and framework-light.
- Keep dashboard/UI dependencies out of proxy and policy crates.
- Avoid introducing persistence or distributed coordination in v0.1.0.
- Prefer mature Rust HTTP ecosystem libraries over custom protocol code.
- Add dependencies only where they reduce implementation risk or test burden.

## Security Boundaries

- The reverse proxy path sees sensitive request data; downstream state must receive only redacted or derived values unless required for origin forwarding.
- Cache entries can contain response bodies, but only after policy allows storage.
- Observation metadata must never contain raw `Authorization`, `Cookie`, `Set-Cookie`, request bodies, or high-risk query values.
- Metrics labels must use `route_id`, `method`, `decision`, and `status_class`, not raw path or query.

## Open Questions

- Whether dashboard static assets are compiled into the binary or served from a development bundle for early milestones.
- Whether `routes`, `explain`, and `purge` CLI commands communicate with a running local admin API or read process-local state only in the same process. For v0.1.0, a local admin API is the practical choice.
- Whether HTTP/2 is included opportunistically through the chosen stack. It is not required for v0.1.0 acceptance.
