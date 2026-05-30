# Testing and Release

Status: design draft
Target release: `v0.3.2`

## Testing Strategy

v0.3.2 is a behavior-preserving refactor, so the test plan emphasizes characterization and compatibility rather than new feature coverage.

## Baseline Characterization

Before starting implementation, capture the current passing state:

```bash
cargo fmt --all --check
cargo test --workspace
cargo test -p kubio-proxy --features experimental-http3
cargo test -p kubio-transport --features experimental-http3
cargo test -p kubio-bench --features experimental-http3
cargo test -p kubio-cli --features experimental-http3
```

If any baseline command fails before refactoring starts, record it in the implementation notes and avoid hiding it inside the refactor.

## Per-Crate Gates

After splitting each crate, run its focused tests:

```bash
cargo test -p kubio-core
cargo test -p kubio-policy
cargo test -p kubio-store
cargo test -p kubio-observe
cargo test -p kubio-telemetry
cargo test -p kubio-dashboard
cargo test -p kubio-transport
cargo test -p kubio-proxy
cargo test -p kubio-cli
cargo test -p kubio-bench
```

For HTTP/3-owning crates, also run:

```bash
cargo test -p kubio-transport --features experimental-http3
cargo test -p kubio-proxy --features experimental-http3
cargo test -p kubio-cli --features experimental-http3
cargo test -p kubio-bench --features experimental-http3
```

## Full Release Gate

Before declaring v0.3.2 ready:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p kubio-transport --features experimental-http3
cargo test -p kubio-proxy --features experimental-http3
cargo test -p kubio-cli --features experimental-http3
cargo test -p kubio-bench --features experimental-http3
bash examples/bench/local_smoke.sh
cargo run -p kubio-bench -- --protocol h1 --output json
cargo run -p kubio-bench -- --protocol h2 --output json
cargo run -p kubio-bench --features experimental-http3 -- --protocol h3 --output json
```

Optional but recommended for release candidates:

```bash
cargo clippy -p kubio-transport --features experimental-http3 --all-targets -- -D warnings
cargo clippy -p kubio-proxy --features experimental-http3 --all-targets -- -D warnings
cargo clippy -p kubio-cli --features experimental-http3 --all-targets -- -D warnings
cargo clippy -p kubio-bench --features experimental-http3 --all-targets -- -D warnings
```

## Compatibility Checks

Reviewers should verify:

- Public crate-root imports from v0.3.1 still compile.
- `examples/kubio.yml` and v0.3/v0.3.1 examples still parse.
- CLI help text has not intentionally changed.
- Dashboard API JSON field names are unchanged.
- Prometheus metric names and labels are unchanged.
- Benchmark JSON field names are unchanged.
- `experimental-http3` code remains absent from default builds except guarded public stubs.
- No new dependency was added.

## Test Placement

Guidelines:

- Move unit tests into the module that owns the tested logic.
- Keep cross-crate HTTP proxy integration tests under `crates/kubio-proxy/tests`.
- Keep benchmark smoke scripts under `examples/bench`.
- If a test needs many private internals after the split, prefer testing through an existing public or `pub(crate)` function instead of making production helpers public.

## Release Notes

The release notes should frame v0.3.2 as a maintainability release:

- Source layout split into cohesive modules.
- No intentional operator-facing behavior changes.
- Public crate-root APIs preserved.
- Full default and HTTP/3 feature test gates passed.

Avoid listing internal file moves exhaustively in release notes. Link to the design or development docs for source layout details.
