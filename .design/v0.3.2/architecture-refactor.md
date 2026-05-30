# Architecture Refactor

Status: design draft
Target release: `v0.3.2`

## Goals

v0.3.2 restructures source files inside the existing workspace crates. It should make code ownership clear without changing runtime behavior or public crate usage.

## Refactor Rules

1. Keep crate names and workspace membership unchanged.
2. Keep dependency lists unchanged unless a compile error proves an existing dependency was accidentally unused.
3. Keep crate-root public exports compatible.
4. Prefer moving existing code over rewriting it.
5. Prefer module-level `#[cfg(feature = "experimental-http3")]` over scattered item-level `cfg` where the code is already isolated.
6. Move tests only when the test's target module is moved. Do not weaken assertions.
7. Keep integration tests where they are.
8. Do not rename metrics, event variants, config fields, CLI flags, dashboard API fields, or benchmark JSON fields.

## Crate Root Pattern

Library crate roots should converge on this pattern:

```rust
//! Crate-level summary.

mod cache_key;
mod config;
mod headers;

pub use cache_key::{build_cache_key, CacheKey, CacheKeyHash};
pub use config::{EffectiveConfig, RedactedConfig};
```

Rules:

- Re-export public names from the crate root if they were public before.
- Keep modules private unless there is a concrete caller need.
- Use `pub(crate)` for cross-module helpers.
- Avoid `pub mod` for implementation namespaces unless the namespace itself is part of the intended API.

## Target Module Layout

### `kubio-core`

Purpose: shared types, config DTOs, deterministic normalization, hashing, and parsing helpers.

Target:

```text
src/lib.rs
src/mode.rs
src/protocol.rs
src/config/mod.rs
src/config/server.rs
src/config/policy.rs
src/config/storage.rs
src/config/route.rs
src/cache_key.rs
src/decision.rs
src/normalization.rs
src/headers.rs
src/hash.rs
src/metrics.rs
src/parsing.rs
```

Notes:

- `Mode`, `FreshnessProfile`, `HttpProtocol`, and `OriginProtocolPreference` can move out first because they are low-coupling enums.
- Config structs can live under `config/`, but `EffectiveConfig`, `ServerConfig`, `PolicyConfig`, and route hint types must remain re-exported from `kubio_core`.
- `RouteId` may sit with route config or in `route.rs`; choose the placement that minimizes import churn.
- Cache key helpers should stay independent of policy and observer crates.

### `kubio-policy`

Purpose: deterministic safety policy and signal classification.

Target:

```text
src/lib.rs
src/engine.rs
src/signals.rs
src/classes.rs
src/decision.rs
src/headers.rs
```

Notes:

- `PolicyEngine` remains the crate-root primary export.
- `RequestSignals`, `ResponseSignals`, `PolicyDecision`, `CacheControlClass`, `VaryClass`, and `ContentTypeClass` remain exported.
- Header parsing helpers should remain private unless already public.

### `kubio-store`

Purpose: cache store trait, memory store, disk store, cache entries, purge, metrics, and errors.

Target:

```text
src/lib.rs
src/entry.rs
src/error.rs
src/memory.rs
src/disk.rs
src/metadata.rs
src/purge.rs
src/metrics.rs
```

Notes:

- `CacheStore`, `MemoryStore`, `DiskStore`, `CacheEntry`, `StoreError`, `StoreStats`, `PurgeSelector`, and `PurgeResult` remain re-exported.
- Disk metadata helpers can be private to `disk.rs` or `metadata.rs`; choose based on final file size.
- Keep async disk task behavior unchanged.

### `kubio-observe`

Purpose: process-local observation state, route/key stats, events, counters, and snapshots.

Target:

```text
src/lib.rs
src/observer.rs
src/state.rs
src/records.rs
src/events.rs
src/protocol.rs
src/snapshot.rs
src/query.rs
src/latency.rs
```

Notes:

- `Observer` remains the primary crate-root export.
- Snapshot DTOs remain public and crate-root re-exported because dashboard, telemetry, and CLI depend on them.
- `RouteStats`, `QueryParamStats`, and `ObserverInner` should stay private.
- Protocol counters and HTTP/3 event enums can move together to reduce observer file size.
- Snapshot generation can stay as methods on private stats types, but DTO definitions should be easier to find.

### `kubio-telemetry`

Purpose: tracing setup and Prometheus-style metrics rendering.

Target:

```text
src/lib.rs
src/tracing.rs
src/render.rs
src/labels.rs
src/store.rs
src/histogram.rs
```

Notes:

- `init_tracing`, `render_metrics`, and `sanitize_label` remain crate-root exports.
- Metric names and label order must not change.
- Unit tests that assert metric strings should move with the rendering helpers.

### `kubio-dashboard`

Purpose: dashboard routing, HTML pages, JSON APIs, authorization, and models.

Target:

```text
src/lib.rs
src/state.rs
src/router.rs
src/pages.rs
src/api.rs
src/auth.rs
src/html.rs
src/models.rs
```

Notes:

- `DashboardState`, `router`, and `run_dashboard` remain crate-root exports.
- Keep HTML output behavior intentionally unchanged.
- Dashboard API DTOs remain serializable with the same field names.

### `kubio-transport`

Purpose: TCP/TLS/HTTP/1.1/HTTP/2 serving, HTTP/3 serving, origin client builders, and TLS helpers.

Target:

```text
src/lib.rs
src/http12.rs
src/origin.rs
src/tls.rs
src/http.rs
src/http3/mod.rs
src/http3/server.rs
src/http3/client.rs
src/http3/body.rs
src/http3/config.rs
```

Notes:

- Default-build APIs remain available from `kubio_transport`.
- HTTP/3 modules should be gated at `mod http3` where possible.
- `experimental_http3_build_enabled`, `serve_http3_router`, `Http3OriginClient`, `Http3OriginResponse`, `Http3ServerTelemetry`, and `Http3ServerEvent` remain available under the same feature expectations.
- Keep TLS loading behavior and ALPN lists unchanged.

### `kubio-proxy`

Purpose: protocol-neutral proxy runtime, cache flow, origin I/O, route hints, revalidation, stale handling, Alt-Svc, and response construction.

Target:

```text
src/lib.rs
src/state.rs
src/router.rs
src/handler.rs
src/route_hints.rs
src/in_flight.rs
src/origin.rs
src/cache.rs
src/revalidation.rs
src/response.rs
src/alt_svc.rs
src/query.rs
src/headers.rs
src/panic_switch.rs
src/protocol.rs
```

Notes:

- `ProxyState`, `router`, and `run_proxy` remain crate-root exports.
- `handler.rs` may remain the largest file, but origin I/O, response construction, Alt-Svc, query observation, route hint lookup, and stale/revalidation helpers should move out.
- Do not change the request flow in the same patch as a module move.
- `OriginResponse` and `OriginError` should live with origin request execution.
- `ObservedInFlightPermit` should live in `in_flight.rs`.
- `alt_svc_decision` should be isolated because it has a clear config-only decision surface.

### `kubio-cli`

Purpose: binary entry point, Clap args, command handlers, config file loading/application/validation, and admin HTTP calls.

Target:

```text
src/main.rs
src/args.rs
src/commands/mod.rs
src/commands/serve.rs
src/commands/routes.rs
src/commands/explain.rs
src/commands/doctor.rs
src/commands/purge.rs
src/config/mod.rs
src/config/file.rs
src/config/apply.rs
src/config/validate.rs
src/config/route.rs
src/admin.rs
src/output.rs
src/shutdown.rs
```

Notes:

- `main.rs` should only initialize tracing, parse args, and dispatch.
- Config validation should be independently navigable because it is safety-critical.
- File config DTOs can stay private under `config/file.rs`.
- Command output should not be intentionally rewritten.

### `kubio-bench`

Purpose: local benchmark runner, managed origin/proxy processes, protocol clients, and report formatting.

Target:

```text
src/main.rs
src/args.rs
src/report.rs
src/origin.rs
src/proxy.rs
src/client.rs
src/h3.rs
```

Notes:

- Keep JSON output stable.
- Gate `h3.rs` behind `experimental-http3`.
- Keep fixture path behavior unchanged.

## Implementation Order

Use a low-risk order:

1. `kubio-core` and `kubio-policy`
2. `kubio-store`
3. `kubio-observe`
4. `kubio-telemetry` and `kubio-dashboard`
5. `kubio-transport`
6. `kubio-proxy`
7. `kubio-cli` and `kubio-bench`

Rationale:

- Core and policy are mostly pure Rust and provide the foundation for later import updates.
- Store and observe have clear internal state boundaries.
- Transport and proxy carry the highest runtime risk, so they should move after leaf modules have stabilized.
- CLI can be last because it imports most of the workspace and benefits from settled crate-root re-exports.

## Compatibility Requirements

The following imports should still compile after the refactor:

```rust
use kubio_core::{EffectiveConfig, RouteId, DecisionReason};
use kubio_dashboard::{run_dashboard, DashboardState};
use kubio_observe::{Observer, ObserverSnapshot, ProtocolCounts};
use kubio_policy::{PolicyDecision, PolicyEngine, RequestSignals, ResponseSignals};
use kubio_proxy::{run_proxy, router, ProxyState};
use kubio_store::{CacheEntry, CacheStore, DiskStore, MemoryStore, PurgeSelector};
use kubio_telemetry::{init_tracing, render_metrics, sanitize_label};
use kubio_transport::{origin_client_builder, serve_http12_router};
```

Feature-gated HTTP/3 imports should still compile when `experimental-http3` is enabled for the owning package:

```rust
use kubio_transport::{
    serve_http3_router, Http3OriginClient, Http3OriginResponse, Http3ServerTelemetry,
};
```

## Review Strategy

Each implementation patch should state which category it belongs to:

- Move only.
- Move plus import updates.
- Move plus visibility adjustment.
- Move plus test relocation.

Any patch that changes logic, constants, config defaults, or output text should be treated as outside the v0.3.2 goal unless it fixes a bug found during the refactor and is documented separately.
