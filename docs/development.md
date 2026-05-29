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

Useful commands:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --all-targets --all-features -- -D warnings
```

The proxy path should fail open to origin on internal errors. Add tests before changing policy rules.
