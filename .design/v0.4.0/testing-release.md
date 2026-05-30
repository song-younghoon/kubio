# Testing and Release Plan

Status: implemented scope
Target release: `v0.4.0`

v0.4.0 changes distribution behavior, so the release gate must prove that
install and update paths work from release artifacts without weakening existing
proxy behavior.

## 1. Existing Gates

Keep the current default gates:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --features experimental-http3
REQUESTS=10 MODE=auto bash examples/bench/local_smoke.sh
bash examples/bench/baseline_scenarios.sh
cargo run -p kubio-bench -- --requests 20 --protocol h1 --output json --fail-on-budget
cargo run -p kubio-bench -- --requests 20 --protocol h2 --output json --fail-on-budget
cargo run -p kubio-bench --features experimental-http3 -- --requests 20 --protocol h3 --output json --fail-on-budget
```

The install/update work must not require changes to cache safety gates.

## 2. Installer Static Checks

Required:

```bash
bash -n install.sh
```

Recommended when available:

```bash
shellcheck install.sh
```

`shellcheck` can be an advisory local check unless it is added to CI
deliberately.

## 3. Installer Unit-Style Scenarios

The installer should be structured so tests can override the download base:

```bash
KUBIO_DOWNLOAD_BASE_URL=file:///tmp/kubio-dist KUBIO_INSTALL_DIR=/tmp/kubio-bin bash install.sh
```

Scenarios:

- standard artifact installs as `kubio`;
- HTTP/3 experimental artifact installs as `kubio`;
- `KUBIO_VERSION=v0.4.0` chooses the pinned URL shape;
- missing `SHA256SUMS` fails;
- mismatched checksum fails before install;
- unsupported `KUBIO_FLAVOR` fails before download;
- unwritable install directory fails without partial install;
- install manifest is written when config directory is writable;
- missing manifest write does not fail an otherwise successful install.

## 4. Update Unit Tests

Add Rust tests for:

- tag parsing and ordering;
- malformed tag rejection;
- artifact name selection;
- flavor precedence;
- unsupported platform handling;
- update-check cache expiration;
- `ETag` request header handling;
- development executable path refusal;
- stdout/stderr separation for explicit and ambient paths.

Network behavior should be tested with local HTTP fixtures or mocked clients,
not the live GitHub API.

## 5. Update Integration Scenarios

Use staged local artifacts and a local metadata endpoint where possible:

- `kubio update --check` reports current when latest equals current;
- `kubio update --check` reports available when latest is newer;
- `kubio update --check` returns non-zero for explicit unreachable metadata;
- ambient `serve` check does not delay listener startup;
- `kubio update` downloads, verifies, and replaces a test install;
- checksum mismatch leaves old binary in place;
- `--version` installs a pinned artifact;
- `--flavor http3-experimental` selects the experimental artifact.

## 6. Release Workflow Gates

The release workflow should:

1. Build standard and HTTP/3 experimental Linux artifacts.
2. Build `SHA256SUMS`.
3. Run `bash -n install.sh`.
4. Stage artifacts into `dist/`.
5. Install standard artifact from staged `dist/` into a temp directory.
6. Run the installed `kubio --help`.
7. Run the installed `kubio --version`.
8. Install HTTP/3 experimental artifact from staged `dist/`.
9. Run the installed experimental `kubio --help`.
10. Smoke-test `kubio update --check` against a fixture endpoint.
11. Upload `install.sh` with release assets.

Tagged releases should still publish:

- both binaries;
- `SHA256SUMS`;
- installer;
- benchmark JSON artifacts.

## 7. Manual Release Candidate Checks

Before tagging v0.4.0:

```bash
tmpdir="$(mktemp -d)"
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR="$tmpdir/bin" bash
"$tmpdir/bin/kubio" --version
"$tmpdir/bin/kubio" update --check
```

After the GitHub Release is published:

```bash
tmpdir="$(mktemp -d)"
curl -fsSL https://github.com/song-younghoon/kubio/releases/download/v0.4.0/install.sh | KUBIO_INSTALL_DIR="$tmpdir/bin" bash
"$tmpdir/bin/kubio" doctor --to http://localhost:3000
```

Use a local origin for `doctor` or skip that command if no origin is running.

## 8. Release Notes

Release notes should call out:

- one-command install;
- supported platform: Linux x86_64 only;
- install directory defaults and `PATH` hint;
- `KUBIO_VERSION`, `KUBIO_INSTALL_DIR`, and `KUBIO_FLAVOR`;
- `kubio update --check`;
- `kubio update`;
- update-check opt-out environment variables;
- checksum verification;
- no automatic background updates.

## 9. Blockers

Block the release if:

- installer can partially overwrite an existing binary after checksum failure;
- unsupported platforms attempt a download;
- update notices print to stdout during existing commands;
- `kubio serve` waits on the update network call before starting listeners;
- release assets are missing checksums;
- self-update can replace a `target/debug` or `target/release` development
  binary without `--force`;
- existing default or HTTP/3 feature tests regress.
