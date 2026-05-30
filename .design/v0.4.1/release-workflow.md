# Release Workflow

Status: implemented; release workflow validated.
Target release: `v0.4.1`

## 1. Workflow Shape

Refactor `.github/workflows/release.yml` from a single Linux publishing job into
platform build jobs plus a final publish job:

```text
linux-x86_64
macos-arm64
linux-arm64
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
runs-on: ubuntu-latest
```

Build environment:

```text
Rust aarch64-unknown-linux-gnu target plus GNU aarch64 cross linker
```

The job cross-compiles Linux arm64 artifacts on a GitHub-hosted Linux runner.
This avoids the self-hosted macOS runner's Docker Hub cold-pull bottleneck while
also avoiding a full Rust build under QEMU. QEMU is used only for short
`--help` and `--version` execution smoke tests after the artifact has already
been built.

Preflight:

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt-get install gcc-aarch64-linux-gnu libc6-dev-arm64-cross qemu-user file
```

Preferred implementation path:

1. install the Rust `aarch64-unknown-linux-gnu` target;
2. install `gcc-aarch64-linux-gnu`, `qemu-user`, and `file`;
3. set `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER` to
   `aarch64-linux-gnu-gcc`;
4. run `cargo build --release -p kubio-cli --target
   aarch64-unknown-linux-gnu`;
5. copy the output as `kubio-aarch64-unknown-linux-gnu`;
6. run `cargo build --release -p kubio-cli --features experimental-http3
   --target aarch64-unknown-linux-gnu`;
7. copy the output as `kubio-aarch64-unknown-linux-gnu-http3-experimental`;
8. verify the binaries are ELF aarch64 files;
9. run `kubio --version` and `kubio --help` through `qemu-aarch64 -L
   /usr/aarch64-linux-gnu`.

The Docker-on-Apple-Silicon approach remains a reasonable future optimization
once the runner has a pre-warmed or project-owned Linux arm64 build image. It is
not used for v0.4.1 publishing because repeated Docker Hub cold pulls timed out
before reaching the Rust build.

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
needs: [linux-x86_64, linux-arm64, macos-arm64, docker-image]
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
- Linux arm64: ELF validation plus short `qemu-aarch64` startup smoke. Full
  staged install/self-update smoke can move to this job when a native Linux
  arm64 runner is available.

The final publish job should not need to execute every platform binary.
