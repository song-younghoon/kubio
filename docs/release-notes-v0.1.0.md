# v0.1.0 Release Notes Draft

kubio v0.1.0 is the first local-first release.

## Highlights

- HTTP/1.1 reverse proxy for local API traffic.
- Watch, shadow, and auto modes.
- Conservative safety policy for authenticated, cookie, unsafe method, sensitive path, unsafe response header, unsupported Vary, and unstable response cases.
- Shadow validation before automatic reuse.
- In-memory cache with TTL, object size, total size, purge, and eviction counters.
- Local dashboard with overview, routes, route detail, events, config, and JSON APIs.
- Prometheus-compatible metrics with bounded labels and configurable metrics path.
- Configurable origin timeout and deterministic gateway timeout behavior.
- Panic switch file that immediately disables reuse and promotion while keeping origin pass-through active.
- Dockerfile and CI checks for formatting, clippy, tests, supply-chain checks, and Docker build.
- Release workflow for Linux x86_64 binary artifacts, checksums, and Docker image smoke tests.

## Known Limits

- Process-local cache and observation state only.
- No distributed cache consistency.
- No stale-if-error behavior.
- No POST, GraphQL, or mutation reuse.
- No conditional revalidation with ETag or Last-Modified yet.
- Release binary packaging and checksum publishing still need final CI wiring before a public tag.

## Verification

Required local checks:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```
