# kubio v0.4.1 Design Index

Status: released.
Source: v0.4.0 distribution follow-up
Target release: `v0.4.1`

v0.4.1 extends the v0.4.0 install and update work from one Linux target to a
small, explicit set of release platforms. The release remains focused on
distribution quality and operator usability, not new proxy behavior.

The release theme is:

```text
Keep the same install command, but make the release assets match more machines.
```

## Baseline

v0.4.0 ships:

- one-command install through repository-root `install.sh`;
- standard and HTTP/3 experimental Linux x86_64 release artifacts;
- `SHA256SUMS` verification for install and update;
- `kubio update --check` and `kubio update`;
- an install manifest under the user's config directory.

The remaining adoption gap is platform coverage. Users on arm64 Linux hosts and
Apple Silicon macOS currently get an unsupported-platform error even though the
Rust codebase is portable enough to build for those targets.

## Supported Targets

v0.4.1 should support these release targets:

| Host | Rust target triple | Install support | Update support |
| --- | --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | existing | existing |
| Linux arm64 | `aarch64-unknown-linux-gnu` | new | new |
| macOS arm64 | `aarch64-apple-darwin` | new | new |

Unsupported in v0.4.1:

- macOS x86_64;
- Windows;
- Linux armv7 or other non-arm64 ARM variants;
- musl Linux artifacts;
- package managers such as Homebrew, apt, rpm, npm, and cargo-binstall.

## User-Facing Target

The install command should stay unchanged:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

The installer should detect the host and select the matching artifact. Users
should not need to specify the platform for normal installs.

Pinned and customized installs remain:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_VERSION=v0.4.1 bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR=/usr/local/bin bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_FLAVOR=http3-experimental bash
```

After installation:

```bash
kubio --version
kubio update --check
kubio update
```

## Runner Assumption

The repository has a self-hosted Apple Silicon macOS runner available. The
workflow should address it through the labels currently registered on the
runner:

```yaml
runs-on: [self-hosted, macOS, ARM64]
```

If more self-hosted Apple Silicon runners are added later, a repository-specific
custom label can narrow scheduling. The macOS runner should build and smoke test
only; release publishing should happen from a final GitHub-hosted publish job
with `contents: write`.

macOS arm64 should be built natively on the Apple Silicon self-hosted runner.
Linux arm64 should be cross-compiled on GitHub-hosted Linux with the GNU
aarch64 toolchain; QEMU is used only for short `--help` and `--version` smoke
checks, not for the Rust build. Docker-on-Apple-Silicon remains a possible
future optimization after the runner has a pre-warmed or project-owned Linux
arm64 build image, but v0.4.1 should not block on Docker Hub cold pulls.

## Scope

In scope:

- Add Linux arm64 and macOS arm64 release artifacts for both standard and
  HTTP/3 experimental flavors.
- Refactor installer platform detection from a one-target check into a target
  resolver.
- Refactor updater artifact selection so it uses the current host target instead
  of a Linux x86_64 constant.
- Preserve checksum verification and fail-closed replacement behavior.
- Refactor the release workflow so platform jobs build and upload workflow
  artifacts, then a final publish job creates the GitHub Release.
- Run native smoke checks on macOS arm64 through the self-hosted runner.
- Run Linux arm64 cross-build checks and minimal QEMU execution smoke checks on
  GitHub-hosted Linux.
- Update README, install/update docs, deployment docs, roadmap, and release
  notes to describe the new support matrix.

Out of scope:

- New proxy, cache, dashboard, metrics, config, or benchmark behavior.
- Automatic platform fallback to a source build.
- Homebrew formula, signed macOS installer package, notarization, apt/rpm
  packages, Docker multi-arch image publication, or Windows support.
- Changing the public update command surface.
- Stronger supply-chain signing. v0.4.1 continues the v0.4.0 checksum model
  unless a separate signing release is scheduled.

## Documents

- [PRD](PRD.md)
  - Product goals, user experience, non-goals, and success metrics.
- [Platform and Artifact Contract](platform-and-artifact-contract.md)
  - Supported targets, artifact names, checksum behavior, and manifest fields.
- [Installer and Updater Platforms](installer-and-updater-platforms.md)
  - Host detection, checksum command portability, updater target resolution, and
    failure behavior.
- [Release Workflow](release-workflow.md)
  - Linux x86_64, Linux arm64, macOS arm64, artifact aggregation, and publish
    job design.
- [Testing and Release Plan](testing-release.md)
  - Unit, installer, updater, platform smoke, CI, and manual release gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone task breakdown.

## Cross-Cutting Constraints

- The install command remains stable.
- Platform detection must fail before download when the host is unsupported.
- The installer and updater must derive the same artifact name for the same
  target and flavor.
- Every published binary must have a `SHA256SUMS` entry.
- `kubio update` must not replace a binary with an artifact for a different
  platform.
- macOS support is for command-line installation through the shell installer;
  v0.4.1 does not promise signed or notarized app distribution.
- Self-hosted runner jobs should not hold release write permissions or long-lived
  secrets.

## Milestone Map

- M0: Design and platform contract.
- M1: Installer and updater platform resolution.
- M2: Release workflow and artifact aggregation.
- M3: Platform smoke tests and failure cases.
- M4: README, docs, and release notes.
- M5: v0.4.1 release hardening.
