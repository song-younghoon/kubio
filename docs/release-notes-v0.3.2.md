# Release Notes v0.3.2

Status: implemented scope; release packaging pending.

## Highlights

- Refactored crate internals into focused modules without intentional feature,
  configuration, CLI, dashboard, metric, benchmark JSON, or runtime behavior
  changes.
- Preserved existing public crate-root imports through re-exports in library
  crates.
- Reduced binary crate entrypoints: `kubio-cli` and `kubio-bench` now keep
  `main.rs` focused on parse/dispatch/output orchestration.
- Isolated feature-gated HTTP/3 code in transport and benchmark modules while
  preserving the existing `experimental-http3` feature gate.
- Added final source layout documentation in `docs/development.md`.

## Compatibility Notes

- No new dependencies were added for v0.3.2.
- Config file keys, CLI flags, dashboard paths, API response fields, metrics,
  and benchmark report fields are intended to remain unchanged from v0.3.1.
- This release is a maintainability release; it does not change cache safety
  policy, transport defaults, storage formats, or promotion thresholds.

## Benchmark Summary

Loopback smoke checks completed with 20 requests per run.

| Check | Requests | Successes | p95 ms | Budget |
| --- | ---: | ---: | ---: | --- |
| local smoke script | 20 | 20 observed | 1.08 | n/a |
| kubio-bench h1 | 20 | 20 | 0.80 | pass |
| kubio-bench h2 | 20 | 20 | 0.95 | pass |
| kubio-bench h3 | 20 | 20 | 1.01 | pass |

## Verification

Required local checks:

```bash
cargo fmt --all --check
cargo test --workspace
cargo test --workspace --features experimental-http3
REQUESTS=20 bash examples/bench/local_smoke.sh
cargo run -p kubio-bench -- --requests 20 --protocol h1 --output json --fail-on-budget
cargo run -p kubio-bench -- --requests 20 --protocol h2 --output json --fail-on-budget
cargo run -p kubio-bench --features experimental-http3 -- --requests 20 --protocol h3 --output json --fail-on-budget
```

The local smoke benchmark uses loopback ports and may need alternate
`ORIGIN_PORT`, `PROXY_PORT`, or `DASHBOARD_PORT` values if those defaults are
already in use.
