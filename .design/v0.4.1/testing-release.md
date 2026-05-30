# Testing and Release Plan

Status: implemented locally; release workflow validation pending
Target release: `v0.4.1`

v0.4.1 changes the supported release matrix, so tests must prove that platform
selection and artifact publication are correct without weakening v0.4.0 install
and update safety.

## 1. Existing Gates

Keep the Linux x86_64 gates:

```bash
bash -n install.sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --features experimental-http3
bash examples/bench/baseline_scenarios.sh
cargo run -p kubio-bench -- --requests 50 --protocol h1 --output json --fail-on-budget
cargo run -p kubio-bench -- --requests 50 --protocol h2 --output json --fail-on-budget
cargo run -p kubio-bench --features experimental-http3 -- --requests 50 --protocol h3 --output json --fail-on-budget
```

## 2. Installer Unit-Style Scenarios

Add shell test coverage or release-workflow smoke for:

- `Linux:x86_64` resolves `x86_64-unknown-linux-gnu`;
- `Linux:amd64` resolves `x86_64-unknown-linux-gnu`;
- `Linux:aarch64` resolves `aarch64-unknown-linux-gnu`;
- `Linux:arm64` resolves `aarch64-unknown-linux-gnu`;
- `Darwin:arm64` resolves `aarch64-apple-darwin`;
- `Darwin:x86_64` fails before download;
- unsupported Linux architecture fails before download;
- checksum verification works with `sha256sum`;
- checksum verification works with `shasum -a 256`;
- selected standard and HTTP/3 experimental artifact names are correct for
  every supported target.

If the installer remains a standalone shell script without a dedicated test
harness, keep these scenarios in the release workflow with fake `uname` and
staged `file://` artifacts.

## 3. Updater Unit Tests

Add Rust tests for:

- `ReleaseTarget::current()` mapping through injectable OS/ARCH helpers;
- unsupported platform diagnostics;
- target-specific artifact names for both flavors;
- manifest target match;
- manifest target mismatch refusal;
- v0.4.0 Linux x86_64 manifest compatibility;
- checksum verification with all v0.4.1 artifact names.

Network behavior should continue using local fixtures or `file://` overrides.

## 4. Platform Build Gates

Linux x86_64:

- standard build;
- HTTP/3 experimental build;
- native `--help`;
- native `--version`;
- release smoke;
- staged install/update smoke.

Linux arm64:

- standard build inside Docker `linux/arm64` on Apple Silicon;
- HTTP/3 experimental build inside Docker `linux/arm64` on Apple Silicon;
- Linux aarch64 ELF validation;
- container-native `--help` and `--version`;
- staged install and self-update smoke inside the container when practical;
- checksum inclusion.

macOS arm64:

- native standard build;
- native HTTP/3 experimental build;
- native `--help`;
- native `--version`;
- staged install smoke;
- staged self-update smoke.

## 5. Release Asset Verification

Before publishing, verify:

```text
kubio-x86_64-unknown-linux-gnu
kubio-x86_64-unknown-linux-gnu-http3-experimental
kubio-aarch64-unknown-linux-gnu
kubio-aarch64-unknown-linux-gnu-http3-experimental
kubio-aarch64-apple-darwin
kubio-aarch64-apple-darwin-http3-experimental
SHA256SUMS
install.sh
kubio-bench-h1.json
kubio-bench-h2.json
kubio-bench-h3.json
```

Also verify:

- `SHA256SUMS` contains all six binaries;
- no extra `kubio-*` binary assets are present unless explicitly accepted;
- the release workflow uploads from the final aggregated `dist/` directory.

## 6. Manual Release Candidate Checks

Before tagging v0.4.1, run on Linux x86_64:

```bash
tmpdir="$(mktemp -d)"
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR="$tmpdir/bin" bash
"$tmpdir/bin/kubio" --version
"$tmpdir/bin/kubio" update --check
```

Run equivalent checks on the self-hosted macOS arm64 runner:

```bash
tmpdir="$(mktemp -d)"
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR="$tmpdir/bin" bash
"$tmpdir/bin/kubio" --version
"$tmpdir/bin/kubio" update --check
```

For Linux arm64, run the equivalent checks inside Docker `linux/arm64` on the
self-hosted Apple Silicon runner. A native Linux arm64 host can replace this
later if one is added.

## 7. Release Notes

Release notes should call out:

- newly supported Linux arm64 installs;
- newly supported Apple Silicon macOS installs;
- unchanged one-command install path;
- unsupported macOS x86_64 and Windows;
- standard and HTTP/3 experimental artifact names;
- checksum verification still required;
- macOS artifacts are CLI binaries, not notarized app packages.

## 8. Blockers

Block v0.4.1 if:

- any supported target is missing either standard or HTTP/3 experimental binary;
- `SHA256SUMS` misses any published binary;
- macOS installer requires GNU `sha256sum`;
- unsupported hosts attempt a binary download;
- `kubio update` can choose an artifact for a different platform;
- the macOS self-hosted runner publishes releases directly;
- existing Linux x86_64 release gates regress.
