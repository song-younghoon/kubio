# Development

Workspace crates:

- `kubio-cli`: CLI and process lifecycle.
- `kubio-core`: shared types and normalization helpers.
- `kubio-policy`: deterministic safety policy.
- `kubio-observe`: route stats and shadow validation state.
- `kubio-store`: cache store trait, memory store, and disk store.
- `kubio-dashboard`: local dashboard and APIs.
- `kubio-telemetry`: logging and metrics rendering.
- `kubio-transport`: TCP/TLS/QUIC transport adapters.
- `kubio-proxy`: reverse proxy runtime.
- `kubio-bench`: local benchmark runner with h1/h2/h3 scenarios.

Source layout:

- `kubio-core` keeps shared API types in focused modules: config, mode,
  protocol, route identity, cache keys, policy decisions, normalization,
  hashing, header redaction, metrics, and parsers. Public names are re-exported
  from `lib.rs`.
- `kubio-policy` separates classification types, input signals, policy
  decisions, cache-control/header helpers, and `PolicyEngine`.
- `kubio-store` separates the store trait, entries, errors, purge selectors,
  stats, memory storage, disk storage, and disk metadata.
- `kubio-observe` separates observer state, event records, snapshots, route
  and query state, protocol counters, and latency aggregation.
- `kubio-telemetry` separates tracing setup, metric labels, text rendering,
  store metrics, histograms, and Prometheus output rendering.
- `kubio-dashboard` separates state, router wiring, JSON APIs, auth helpers,
  HTML pages, HTML escaping, and response models.
- `kubio-transport` separates HTTP/1.1 and HTTP/2 serving, origin client
  builders, TLS helpers, and feature-gated HTTP/3 code under `http3/`.
- `kubio-proxy` keeps the request path in `handler.rs` and separates proxy
  state, router startup, route hints, in-flight accounting, origin forwarding,
  cache freshness, revalidation, response construction, query observation,
  headers, and Alt-Svc logic.
- `kubio-cli` keeps `main.rs` as init/parse/dispatch only. Clap args live in
  `args.rs`, command handlers in `commands/`, and config file loading,
  application, and validation in `config/`.
- `kubio-bench` keeps `main.rs` as parse/output orchestration. Args, reports,
  managed origin/proxy helpers, protocol clients, and the benchmark runner live
  in separate modules; HTTP/3 client code is feature-gated in `h3.rs`.

Useful commands:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --all-targets --all-features -- -D warnings
```

Release install/update checks:

```bash
bash -n install.sh
cargo run -p kubio-cli -- update --check
```

Local smoke benchmark:

```bash
bash examples/bench/local_smoke.sh
cargo run -p kubio-bench -- --protocol h1 --output json
cargo run -p kubio-bench -- --protocol h2 --output json
cargo run -p kubio-bench --features experimental-http3 -- --protocol h3 --output json
```

The proxy path should fail open to origin on internal errors. Add tests before changing policy rules.
