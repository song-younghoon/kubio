# v0.3.2 Implementation Tasks

Status: design draft
Target release: `v0.3.2`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.3.2 scope

## Current Implementation Snapshot

v0.3.1 has useful workspace crate boundaries, but each crate still concentrates most implementation into a single source file. v0.3.2 targets a behavior-preserving module split.

## M0: Design and Baseline

Goal: lock the refactor plan and baseline current behavior before moving code.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.3.2 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add architecture refactor plan.
- [x] M0.1.4 Add testing and release plan.
- [x] M0.1.5 Add implementation task breakdown.

Acceptance:

- Roadmap includes v0.3.2 as a structure-only refactor release.
- Scope explicitly forbids feature and behavior changes.

### M0.2 Baseline Characterization

- [ ] M0.2.1 Run default workspace tests.
- [ ] M0.2.2 Run HTTP/3 feature tests for `kubio-transport`.
- [ ] M0.2.3 Run HTTP/3 feature tests for `kubio-proxy`.
- [ ] M0.2.4 Run HTTP/3 feature tests for `kubio-cli`.
- [ ] M0.2.5 Run HTTP/3 feature tests for `kubio-bench`.
- [ ] M0.2.6 Record any pre-existing failures.

Acceptance:

- The implementation branch has a known baseline.
- Any pre-existing failure is documented separately from the refactor.

## M1: Low-Risk Leaf Crate Splits

Goal: split pure or mostly pure crates first.

### M1.1 `kubio-core`

- [ ] M1.1.1 Move modes and protocol enums into focused modules.
- [ ] M1.1.2 Move config structs into `config/`.
- [ ] M1.1.3 Move cache key helpers into `cache_key.rs`.
- [ ] M1.1.4 Move decision/reason/validator types into `decision.rs`.
- [ ] M1.1.5 Move normalization, header, hash, metric, and parsing helpers into focused modules.
- [ ] M1.1.6 Re-export existing public names from `lib.rs`.
- [ ] M1.1.7 Move unit tests with their target modules.

Acceptance:

- `cargo test -p kubio-core` passes.
- Existing external imports from `kubio_core` still compile.

### M1.2 `kubio-policy`

- [ ] M1.2.1 Move `PolicyEngine` into `engine.rs`.
- [ ] M1.2.2 Move signal structs into `signals.rs`.
- [ ] M1.2.3 Move classification enums into `classes.rs`.
- [ ] M1.2.4 Move `PolicyDecision` into `decision.rs`.
- [ ] M1.2.5 Move private header/cache-control parsing helpers into `headers.rs`.
- [ ] M1.2.6 Re-export existing public names.

Acceptance:

- `cargo test -p kubio-policy` passes.
- No policy decision behavior changes.

## M2: State, Rendering, and Store Splits

Goal: split crates with clear state and DTO boundaries before touching runtime adapters.

### M2.1 `kubio-store`

- [ ] M2.1.1 Move `CacheEntry` into `entry.rs`.
- [ ] M2.1.2 Move `StoreError` into `error.rs`.
- [ ] M2.1.3 Move memory store implementation into `memory.rs`.
- [ ] M2.1.4 Move disk store implementation and metadata helpers into `disk.rs` or `metadata.rs`.
- [ ] M2.1.5 Move purge and stats types into `purge.rs` and `metrics.rs`.
- [ ] M2.1.6 Re-export existing public store API.

Acceptance:

- `cargo test -p kubio-store` passes.
- Disk store metadata format is unchanged.

### M2.2 `kubio-observe`

- [ ] M2.2.1 Move `Observer` methods into `observer.rs`.
- [ ] M2.2.2 Move private mutable state and route/query stats into `state.rs` and `query.rs`.
- [ ] M2.2.3 Move record DTOs into `records.rs`.
- [ ] M2.2.4 Move event enums and protocol counters into `events.rs` and `protocol.rs`.
- [ ] M2.2.5 Move snapshot DTOs into `snapshot.rs`.
- [ ] M2.2.6 Move latency helpers into `latency.rs`.
- [ ] M2.2.7 Re-export existing public observer API.

Acceptance:

- `cargo test -p kubio-observe` passes.
- Snapshot JSON shape is unchanged.

### M2.3 `kubio-telemetry`

- [ ] M2.3.1 Move tracing setup into `tracing.rs`.
- [ ] M2.3.2 Move metrics rendering into `render.rs`.
- [ ] M2.3.3 Move label sanitization into `labels.rs`.
- [ ] M2.3.4 Move store metrics and histogram helpers into focused modules.
- [ ] M2.3.5 Re-export existing public telemetry API.

Acceptance:

- `cargo test -p kubio-telemetry` passes.
- Metrics output is unchanged.

### M2.4 `kubio-dashboard`

- [ ] M2.4.1 Move dashboard state and router wiring into `state.rs` and `router.rs`.
- [ ] M2.4.2 Move HTML pages into `pages.rs`.
- [ ] M2.4.3 Move JSON APIs into `api.rs`.
- [ ] M2.4.4 Move authorization helpers into `auth.rs`.
- [ ] M2.4.5 Move HTML/model helpers into `html.rs` and `models.rs`.
- [ ] M2.4.6 Re-export existing public dashboard API.

Acceptance:

- `cargo test -p kubio-dashboard` passes.
- Dashboard paths and API fields are unchanged.

## M3: Transport and Proxy Splits

Goal: split runtime-heavy code without changing request behavior.

### M3.1 `kubio-transport`

- [ ] M3.1.1 Move HTTP/1.1 and HTTP/2 serving into `http12.rs`.
- [ ] M3.1.2 Move origin client builder helpers into `origin.rs`.
- [ ] M3.1.3 Move TLS loading and ALPN helpers into `tls.rs`.
- [ ] M3.1.4 Move HTTP/3 server/client/body/config code under `http3/`.
- [ ] M3.1.5 Re-export default and feature-gated public transport APIs.

Acceptance:

- `cargo test -p kubio-transport` passes.
- `cargo test -p kubio-transport --features experimental-http3` passes.

### M3.2 `kubio-proxy`

- [ ] M3.2.1 Move `ProxyState` and construction into `state.rs`.
- [ ] M3.2.2 Move router and listener startup into `router.rs`.
- [ ] M3.2.3 Move route hint lookup into `route_hints.rs`.
- [ ] M3.2.4 Move in-flight permit accounting into `in_flight.rs`.
- [ ] M3.2.5 Move origin request execution and fallback into `origin.rs`.
- [ ] M3.2.6 Move cache entry response/freshness helpers into `cache.rs` and `revalidation.rs`.
- [ ] M3.2.7 Move response construction into `response.rs`.
- [ ] M3.2.8 Move Alt-Svc decision logic into `alt_svc.rs`.
- [ ] M3.2.9 Move query observation and header/protocol helpers into focused modules.
- [ ] M3.2.10 Keep the main request flow in `handler.rs`.
- [ ] M3.2.11 Re-export `ProxyState`, `router`, and `run_proxy`.

Acceptance:

- `cargo test -p kubio-proxy` passes.
- `cargo test -p kubio-proxy --features experimental-http3` passes.
- Existing integration tests remain in `crates/kubio-proxy/tests`.

## M4: Binary Crate Splits

Goal: make command and benchmark code navigable while preserving outputs.

### M4.1 `kubio-cli`

- [ ] M4.1.1 Move Clap args into `args.rs`.
- [ ] M4.1.2 Move command handlers into `commands/`.
- [ ] M4.1.3 Move config file DTOs into `config/file.rs`.
- [ ] M4.1.4 Move config application into `config/apply.rs`.
- [ ] M4.1.5 Move config validation into `config/validate.rs`.
- [ ] M4.1.6 Move admin HTTP helpers, output helpers, and shutdown handling into focused modules.
- [ ] M4.1.7 Leave `main.rs` as init, parse, and dispatch only.

Acceptance:

- `cargo test -p kubio-cli` passes.
- `cargo test -p kubio-cli --features experimental-http3` passes.
- CLI output changes are not intentional.

### M4.2 `kubio-bench`

- [ ] M4.2.1 Move args and report types into `args.rs` and `report.rs`.
- [ ] M4.2.2 Move managed origin/proxy helpers into `origin.rs` and `proxy.rs`.
- [ ] M4.2.3 Move protocol clients into `client.rs` and feature-gated `h3.rs`.
- [ ] M4.2.4 Keep JSON output stable.

Acceptance:

- `cargo test -p kubio-bench` passes.
- `cargo test -p kubio-bench --features experimental-http3` passes.
- Benchmark JSON shape is unchanged.

## M5: Documentation and Release Hardening

Goal: ship the refactor with clear source-layout documentation and full compatibility checks.

- [ ] M5.1 Update `docs/development.md` with final module layout.
- [ ] M5.2 Add `docs/release-notes-v0.3.2.md`.
- [ ] M5.3 Run full default release gate.
- [ ] M5.4 Run HTTP/3 feature release gate.
- [ ] M5.5 Run local smoke benchmark.
- [ ] M5.6 Confirm no new dependencies were added.
- [ ] M5.7 Confirm no public behavior changes are called out as part of v0.3.2.

Acceptance:

- v0.3.2 is releasable as a maintainability-only source structure update.
