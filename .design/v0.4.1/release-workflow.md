# Release Workflow

Status: implemented locally; release workflow validation pending
Target release: `v0.4.1`

## 1. Workflow Shape

Refactor `.github/workflows/release.yml` from a single Linux publishing job into
platform build jobs plus a final publish job:

```text
linux-x86_64
macos-arm64
linux-arm64-container
docker-image
publish-release
```

Only `publish-release` needs `contents: write`. Build jobs should use
`contents: read` and upload workflow artifacts.

## 2. Linux x86_64 Job

Runner:

```yaml
runs-on: ubuntu-latest
```

Responsibilities:

- run existing release quality gates;
- build standard binary;
- run `examples/bench/release_smoke.sh`;
- build HTTP/3 experimental binary;
- run `--help` and `--version` on both artifacts;
- produce benchmark JSON artifacts;
- upload:
  - `kubio-x86_64-unknown-linux-gnu`;
  - `kubio-x86_64-unknown-linux-gnu-http3-experimental`;
  - `kubio-bench-h1.json`;
  - `kubio-bench-h2.json`;
  - `kubio-bench-h3.json`.

This job remains the canonical deep validation gate.

## 3. Linux arm64 Job

Runner:

```yaml
runs-on: [self-hosted, macOS, ARM64]
```

Build environment:

```text
Docker container with platform linux/arm64
```

The job builds Linux arm64 artifacts inside Docker on the Apple Silicon runner.
This uses arm64 Linux userspace on arm64 hardware through Docker's Linux VM,
which should be much faster and more faithful than running arm64 binaries under
QEMU on an x86 Linux runner.

Preflight:

```bash
test "$(uname -s)" = "Darwin"
test "$(uname -m)" = "arm64"
docker version --format '{{.Server.Os}}/{{.Server.Arch}}'
```

Preferred implementation path:

1. pull a small stable Rust `linux/arm64` build image;
2. mount persistent runner-local Cargo registry and target caches into the
   container;
3. run `cargo build --release -p kubio-cli`;
4. copy the output as `kubio-aarch64-unknown-linux-gnu`;
5. run `cargo build --release -p kubio-cli --features experimental-http3`;
6. copy the output as `kubio-aarch64-unknown-linux-gnu-http3-experimental`;
7. run `kubio --version` and `kubio --help` inside the same container;
8. run staged installer/update smoke inside the same container when practical.

Suggested base image:

```text
rust:1-slim-bookworm or a project-owned pinned Rust linux/arm64 image
```

Using a pinned project-owned image is preferable once the workflow stabilizes,
because it avoids reinstalling Rust and system packages on every release run and
keeps the glibc baseline explicit.

## 4. macOS arm64 Job

Runner:

```yaml
runs-on: [self-hosted, macOS, ARM64]
```

Preflight:

```bash
test "$(uname -s)" = "Darwin"
test "$(uname -m)" = "arm64"
```

Responsibilities:

- install/use the stable Rust toolchain;
- run `cargo test -p kubio-cli` at minimum;
- build standard binary;
- build HTTP/3 experimental binary;
- run `--help` and `--version` on both artifacts;
- run a staged installer smoke for the standard artifact;
- run a staged self-update smoke using `file://` artifacts;
- upload:
  - `kubio-aarch64-apple-darwin`;
  - `kubio-aarch64-apple-darwin-http3-experimental`.

The macOS runner should not run the final `gh release create/upload` command.

## 5. Artifact Aggregation

Each platform job uploads workflow artifacts. `publish-release` downloads them
into a single `dist/` directory and computes:

```bash
sha256sum kubio-* > SHA256SUMS
```

On the Linux publish runner, `sha256sum` is available. The final `SHA256SUMS`
file covers Linux and macOS binaries.

Expected release binary count:

```text
6
```

Expected install assets:

```text
6 binaries + SHA256SUMS + install.sh
```

Expected auxiliary assets:

```text
kubio-bench-h1.json
kubio-bench-h2.json
kubio-bench-h3.json
```

## 6. Publish Job

Runner:

```yaml
runs-on: ubuntu-latest
needs: [linux-x86_64, linux-arm64-container, macos-arm64, docker-image]
```

Responsibilities:

- download workflow artifacts;
- run `bash -n install.sh`;
- verify every expected asset path exists;
- compute `SHA256SUMS`;
- verify `SHA256SUMS` includes every binary exactly once;
- create or update the GitHub Release on tag pushes;
- upload assets with `--clobber` when the release already exists.

For `workflow_dispatch`, the job may build artifacts and upload workflow
artifacts without publishing unless an explicit input requests release upload.

## 7. Concurrency and Runner Hygiene

Add release workflow concurrency:

```yaml
concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false
```

Self-hosted macOS hygiene:

- use `[self-hosted, macOS, ARM64]` for the current registered runner and add a
  repository-specific custom label later if multiple Apple Silicon runners are
  available;
- avoid repository secrets in the macOS job;
- rely on ephemeral checkout state from `actions/checkout`;
- run preflight commands that fail fast if the runner is not Apple Silicon;
- keep publish credentials in the final GitHub-hosted job only.

## 8. Installer Smoke Placement

Run staged installer smoke in platform jobs where the binary can execute:

- Linux x86_64: native standard and HTTP/3 install smoke.
- macOS arm64: native standard install and self-update smoke.
- Linux arm64: Docker `linux/arm64` standard install and self-update smoke on
  the Apple Silicon runner.

The final publish job should not need to execute every platform binary.
