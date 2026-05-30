# Release Notes v0.4.0

Status: implemented scope; release publishing pending.

## Highlights

- Added repository-root `install.sh` for one-command Linux x86_64 installs.
- Added checksum verification for installed and updated release artifacts.
- Added `kubio update --check` to report when a newer stable release exists.
- Added `kubio update` for verified self-update of release binaries.
- Added best-effort, rate-limited update notices for `kubio serve`, with
  command and environment opt-outs.
- Refreshed README, getting-started, deployment, and install/update docs around
  the released-binary path.

## Compatibility Notes

- v0.4.0 installer support is Linux x86_64 only.
- Standard and HTTP/3 experimental release artifact names remain:
  `kubio-x86_64-unknown-linux-gnu` and
  `kubio-x86_64-unknown-linux-gnu-http3-experimental`.
- Proxy behavior, safety policy, cache semantics, dashboard APIs, metrics, and
  benchmark JSON output are not intentionally changed.
- Update checks request public GitHub Release metadata only. They do not send
  route, origin, cache, dashboard, request, or config data.

## Benchmark Summary

Loopback smoke checks completed with 20 requests per benchmark run.

| Check | Requests | Successes | p95 ms | Budget |
| --- | ---: | ---: | ---: | --- |
| local smoke script | 20 | 20 observed | 1.02 | n/a |
| kubio-bench h1 | 20 | 20 | 0.76 | pass |
| kubio-bench h2 | 20 | 20 | 0.98 | pass |
| kubio-bench h3 | 20 | 20 | 1.04 | pass |

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

Pinned install:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_VERSION=v0.4.0 bash
```

## Update

```bash
kubio update --check
kubio update
```

Disable ambient update notices:

```bash
KUBIO_UPDATE_CHECK=off kubio serve --to http://localhost:3000
kubio serve --no-update-check --to http://localhost:3000
```

## Verification

Required local checks:

```bash
bash -n install.sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --features experimental-http3
REQUESTS=20 bash examples/bench/local_smoke.sh
cargo run -p kubio-bench -- --requests 20 --protocol h1 --output json --fail-on-budget
cargo run -p kubio-bench -- --requests 20 --protocol h2 --output json --fail-on-budget
cargo run -p kubio-bench --features experimental-http3 -- --requests 20 --protocol h3 --output json --fail-on-budget
```
