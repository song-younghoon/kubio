# Testing and Release

Status: implemented local gates; external interoperability remains environment-dependent
Target release: `v0.3.1`

## Goals

Testing must prove that HTTP/3 works, remains experimental, and does not weaken cache safety.

## Required Gates

Keep existing gates:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

Add feature gates:

```bash
cargo test --workspace --features experimental-http3
cargo run -p kubio-bench -- --scenario smoke --protocol h1 --output json --fail-on-budget
cargo run -p kubio-bench -- --scenario smoke --protocol h2 --output json --fail-on-budget
cargo run -p kubio-bench --features experimental-http3 -- --scenario smoke --protocol h3 --output json --fail-on-budget
```

reqwest HTTP/3 is not used in v0.3.1; upstream HTTP/3 uses direct h3/Quinn.

## Unit Tests

Required:

- HTTP/3 config parsing.
- HTTP/3 feature availability validation.
- TLS cert/key validation.
- HTTP/3 authority allowlist validation.
- `Alt-Svc` value rendering.
- `Alt-Svc` skip reason rendering.
- HTTP/3 limit validation.
- Protocol enum serialization.
- Bounded metric label rendering.
- Replayability checks for upstream fallback.

## Integration Tests

### Downstream HTTP/3

- UDP listener starts when enabled.
- Startup fails clearly when feature is absent.
- Safe GET over HTTP/3 can be observed, promoted, stored, and reused.
- Authorization/Cookie over HTTP/3 is protected and never stored.
- GET body over HTTP/3 is protected.
- `Set-Cookie`, `private`, `no-store`, unsupported `Vary`, and non-200 responses are not stored.
- Revalidation over HTTP/3 preserves validator behavior.
- Stale-if-error over HTTP/3 remains explicit and bounded.
- Malformed pseudo headers reject the stream without cache effects.
- Header section limit rejects the stream without cache effects.
- Cross-protocol cache-key equivalence holds for h1, h2, and h3.

### Alt-Svc

- Emitted only when `advertise: true`, HTTP/3 listener is active, and authority is allowed.
- Not emitted for dashboard/admin responses.
- Not emitted for unconfigured authorities.
- Skip reasons are bounded.

### Upstream HTTP/3

- Preferred HTTP/3 attempts HTTP/3 for HTTPS origins.
- Required HTTP/3 fails clearly when origin cannot speak HTTP/3.
- Preferred HTTP/3 falls back for replayable connection failures when configured.
- Unsafe or non-replayable body fallback is blocked after body streaming could have begun.
- Attempted and final upstream protocol are observable.
- Cache key does not change based on upstream protocol.

### Security and Privacy

- 0-RTT is disabled.
- Server push is disabled.
- QUIC connection IDs and tokens are not stored or labeled.
- Authorization/Cookie/Set-Cookie values are absent from metrics, logs, dashboard APIs, disk files, and benchmark output.
- Raw query values do not appear in protocol metrics.
- TLS private key material never appears in logs or dashboard output.

## Interoperability

Release candidate smoke should include, where available:

```bash
curl --http3 -k https://localhost:8443/api/products
curl --http2 -k https://localhost:8443/api/products
curl -k https://localhost:8443/api/products
```

Optional:

- `h3i` malformed request checks.
- Browser HTTP/3 discovery through `Alt-Svc`.
- Wireshark manual diagnostics with key logs in a local-only test environment.

## Release Artifacts

Publish:

- Source tag.
- Standard Linux binary.
- HTTP/3-enabled Linux binary or standard binary built with `experimental-http3`.
- Docker image with HTTP/3 support documented.
- Checksums.
- Release notes.
- HTTP/3 example config.
- Benchmark JSON artifacts.

## Release Exit Criteria

All must be true:

- Existing v0.1.0 through v0.3.0 safety tests pass.
- HTTP/3 feature tests pass.
- Downstream HTTP/3 safe reuse and protected traffic tests pass.
- Upstream HTTP/3 attempt and fallback tests pass or upstream support is explicitly marked disabled in release notes.
- `Alt-Svc` authority tests pass.
- Interoperability smoke runs or is skipped with a documented environment reason.
- `kubio-bench` release budgets pass on the release runner.
- Docs state the exact support level and feature gate.
