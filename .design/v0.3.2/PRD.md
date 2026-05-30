# PRD: kubio v0.3.2

Document status: design draft
Target release: `v0.3.2`
Core philosophy: **change structure, not behavior**

## 1. Product Summary

kubio v0.3.2 is a maintainability release. It should make the source code easier to navigate and safer to modify by splitting large single-file crates into focused Rust modules.

This release should not add operator-facing features. The user-visible product after v0.3.2 should behave the same as v0.3.1.

## 2. Background

v0.1.0 through v0.3.1 added meaningful product surface:

- safety policy and observation loop
- revalidation and stale-if-error
- disk store
- HTTP/2 and HTTP/3 transport work
- dashboard, metrics, CLI, and benchmark runner

The implementation kept momentum by placing most logic in one source file per crate. That made the early product easier to ship, but it now creates friction:

- Reviewing changes requires scanning unrelated code in the same file.
- Tests sit far from the logic they validate.
- `kubio-proxy` mixes state wiring, Axum routing, cache flow, origin I/O, Alt-Svc, request header logic, stale handling, and query observation.
- `kubio-cli` mixes argument definitions, command handlers, config file models, config application, validation, and output helpers.
- `kubio-observe` mixes mutable recording logic, internal state, event types, snapshot DTOs, protocol counters, and latency helpers.
- Feature-gated HTTP/3 code is harder to reason about when it is interleaved with default-build paths.

v0.3.2 addresses that friction directly.

## 3. Goals

v0.3.2 should:

1. Split each large crate into modules with clear responsibility boundaries.
2. Preserve existing public APIs for library crates through `lib.rs` re-exports.
3. Preserve CLI flags, command output shape, config files, dashboard API JSON, metrics output, and benchmark output.
4. Keep feature flags and dependency graphs unchanged.
5. Move module-specific tests next to their implementation where practical.
6. Make HTTP/3 feature-gated code easier to audit by grouping it under module-level `cfg` boundaries where practical.
7. Add source-layout documentation after the implementation lands.
8. Require default and HTTP/3 feature tests before release.

## 4. Developer Experience

After v0.3.2, a contributor should be able to answer common navigation questions quickly:

```text
Where are config types?             kubio-core/src/config/
Where is cache key normalization?   kubio-core/src/cache_key.rs
Where is the proxy handler flow?    kubio-proxy/src/handler.rs
Where is origin fallback logic?     kubio-proxy/src/origin.rs
Where are observer snapshot DTOs?   kubio-observe/src/snapshot.rs
Where is disk store metadata I/O?   kubio-store/src/disk.rs
Where is HTTP/3 server adapter code? kubio-transport/src/http3/
Where is CLI config validation?     kubio-cli/src/config/validate.rs
```

The expected implementation style is mechanical:

- Introduce a module.
- Move existing related types/functions into it.
- Update imports.
- Re-export existing public names.
- Move or keep tests with minimal edits.
- Run the relevant crate tests immediately.

## 5. Non-Goals

v0.3.2 will not:

- Change proxy runtime behavior.
- Change safety decisions.
- Change cache key construction.
- Change store eviction or persistence semantics.
- Change observer promotion thresholds or snapshot fields.
- Change metrics names, labels, or Prometheus formatting.
- Change CLI command names, flags, error messages intentionally, or output formats.
- Change dashboard routes, API paths, or HTML layout intentionally.
- Introduce async trait boundaries for transport or origin logic.
- Add new dependencies or new workspace crates.

## 6. Product Principles

### 6.1 Public Surface Is Stable

External callers should continue using crate roots such as:

```rust
use kubio_core::EffectiveConfig;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::{CacheStore, MemoryStore};
```

Internal module layout can change, but crate-root public names should remain available.

### 6.2 Behavior Is Proven By Characterization

Before risky moves, run baseline tests for the touched crate. After each crate split, rerun the same tests. The release gate should run both default and HTTP/3 feature coverage.

### 6.3 Module Boundaries Follow Existing Responsibilities

Do not invent a new architecture in this release. The existing workspace boundaries are largely right. v0.3.2 should make those boundaries visible inside each crate.

### 6.4 Visibility Should Tighten By Default

New modules should use `pub(crate)` for implementation helpers. Public re-exports should be deliberate and limited to names already exposed by v0.3.1.

## 7. Success Metrics

The release is successful when:

- No crate keeps more than one broad responsibility in its root source file.
- `lib.rs` files in library crates are mostly module declarations and public re-exports.
- `kubio-proxy`, `kubio-observe`, `kubio-cli`, and `kubio-core` are split into navigable modules.
- Default build tests pass.
- HTTP/3 feature tests pass for crates that own or exercise HTTP/3.
- Metrics, dashboard JSON, CLI output, config parsing, examples, and benchmark output remain compatible.
- Reviewers can audit the refactor as behavior-preserving moves with small visibility/import adjustments.
