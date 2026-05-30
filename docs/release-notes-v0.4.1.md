# Release Notes v0.4.1

Status: implemented locally; release workflow validation pending.

## Highlights

- Added release support for Linux arm64:
  `kubio-aarch64-unknown-linux-gnu`.
- Added release support for Apple Silicon macOS:
  `kubio-aarch64-apple-darwin`.
- Kept the v0.4.0 one-command install and `kubio update` UX unchanged.
- Updated `install.sh` to detect Linux x86_64, Linux arm64, and macOS arm64.
- Updated `kubio update` to select release artifacts from the current host
  target instead of a Linux x86_64 constant.
- Refactored release publishing into platform build jobs plus a final
  aggregated publish job.
- Added Linux arm64 build and smoke checks inside Docker `linux/arm64` on the
  Apple Silicon self-hosted runner.

## Compatibility Notes

- Supported install/update targets:
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu`
  - `aarch64-apple-darwin`
- Unsupported hosts, including Windows and macOS x86_64, fail before artifact
  download.
- Standard and HTTP/3 experimental release artifacts are published for every
  supported target.
- macOS artifacts are checksum-verified CLI binaries, not notarized app
  packages.
- Proxy behavior, safety policy, cache semantics, dashboard APIs, metrics, and
  benchmark JSON output are not intentionally changed.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

Pinned install:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_VERSION=v0.4.1 bash
```

HTTP/3 experimental artifact:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_FLAVOR=http3-experimental bash
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
```

Release workflow checks add Linux x86_64 release smoke, Linux arm64 Docker
smoke, native macOS arm64 smoke, installer smoke, self-update smoke, checksum
verification, and release asset verification.
