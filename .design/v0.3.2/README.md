# kubio v0.3.2 Design Index

Status: design draft
Source: post-v0.3.1 maintainability pass
Target release: `v0.3.2`

This directory defines the v0.3.2 design for restructuring source code without changing kubio behavior.

The release theme is:

```text
Structure-only refactor for maintainable crates.
```

## Baseline

v0.3.1 ships the intended product behavior for this line:

- Safe HTTP/1.1, HTTP/2, and experimental HTTP/3 reverse proxy behavior.
- Policy, cache key, revalidation, stale-if-error, and route hint semantics.
- In-memory and disk stores.
- Observer snapshots, dashboard, metrics, CLI commands, and benchmark runner.
- A transport crate and benchmark crate introduced for HTTP/3.

The current maintainability issue is not the workspace crate split. The workspace already has useful crate boundaries. The issue is that each crate still concentrates most implementation in a single Rust source file:

```text
kubio-proxy      2092 lines
kubio-observe    1877 lines
kubio-cli        1652 lines
kubio-core       1455 lines
kubio-store      1013 lines
kubio-transport   978 lines
kubio-telemetry   941 lines
kubio-policy      621 lines
kubio-dashboard   612 lines
kubio-bench       461 lines
```

v0.3.2 should make these areas easier to change by moving cohesive responsibilities into modules while preserving all public behavior and release artifacts.

## Scope

In scope:

- Split large single-file crate implementations into cohesive modules.
- Keep existing public APIs source-compatible through `lib.rs` re-exports.
- Keep `kubio-cli` and `kubio-bench` command behavior unchanged while moving implementation into internal modules.
- Move unit tests close to the modules they validate.
- Reduce scattered `#[cfg(feature = "experimental-http3")]` blocks by placing them at module boundaries where practical.
- Add development documentation describing the final source layout after implementation.
- Run default and HTTP/3 feature test gates before release.

Out of scope:

- Runtime behavior changes.
- Cache key, policy, store, revalidation, stale-if-error, Alt-Svc, protocol fallback, or observer semantics changes.
- CLI command, flag, config schema, dashboard route, metrics name, label, or JSON shape changes.
- New dependencies.
- New crates.
- Performance tuning.
- Broad API redesign.
- Formatting-only churn outside files touched by the module split.

## Documents

- [PRD](PRD.md)
  - Product goals, non-goals, developer experience, and success metrics.
- [Architecture Refactor](architecture-refactor.md)
  - Target module layout, API preservation rules, and crate-by-crate boundaries.
- [Testing and Release](testing-release.md)
  - Characterization, compatibility gates, feature matrix, and release checklist.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone task breakdown.

## Cross-Cutting Constraints

- The refactor must be behavior-preserving. A reviewer should be able to reason about most changes as moves, re-exports, visibility narrowing, and import updates.
- `lib.rs` files should become crate entry points, not dumping grounds. They should declare modules and re-export the same public types/functions callers already use.
- Prefer `pub(crate)` for newly exposed module internals. Use `pub` only when the symbol was already public or must remain public for an existing external caller.
- Keep module names domain-oriented, not implementation-mechanism-oriented.
- Do not extract abstractions just to reduce line count. Extract around stable responsibilities: config, routing, origin I/O, cache entry handling, observation records, snapshots, metrics rendering, disk persistence, and HTTP transport.
- Avoid introducing generic helper layers between policy/cache/transport unless the current code already implies that boundary.
- Keep tests close to changed code, but do not delete integration tests that validate cross-crate behavior.

## Milestone Map

- M0: Design and baseline characterization.
- M1: Low-risk leaf crate splits.
- M2: Observer, telemetry, dashboard, and store splits.
- M3: Transport and proxy runtime splits.
- M4: CLI and benchmark runner splits.
- M5: Documentation, compatibility audit, and release hardening.
