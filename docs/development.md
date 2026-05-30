# Development

Workspace crates:

- `kubio-cli`: CLI and process lifecycle.
- `kubio-core`: shared types and normalization helpers.
- `kubio-proxy`: reverse proxy runtime.
- `kubio-policy`: deterministic safety policy.
- `kubio-observe`: route stats and shadow validation state.
- `kubio-store`: cache store trait and memory store.
- `kubio-dashboard`: local dashboard and APIs.
- `kubio-telemetry`: logging and metrics rendering.
- `kubio-transport`: TCP/TLS/QUIC transport adapters.
- `kubio-bench`: local benchmark runner with h1/h2/h3 scenarios.

Useful commands:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --all-targets --all-features -- -D warnings
```

Local smoke benchmark:

```bash
bash examples/bench/local_smoke.sh
cargo run -p kubio-bench -- --protocol h1 --output json
cargo run -p kubio-bench -- --protocol h2 --output json
cargo run -p kubio-bench --features experimental-http3 -- --protocol h3 --output json
```

The proxy path should fail open to origin on internal errors. Add tests before changing policy rules.
