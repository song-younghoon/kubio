# kubio v0.4.0 Design Index

Status: implemented scope
Source: distribution and usability planning
Target release: `v0.4.0`

This directory defines the v0.4.0 design and implemented scope for making kubio
easier to install, notice, and update. The release focuses on packaging and
operator experience rather than new proxy behavior.

The release theme is:

```text
Install with one command, know when a newer release exists, and update safely.
```

## Baseline

v0.3.2 leaves kubio with stable local-first proxy behavior and maintainable
crate internals:

- HTTP/1.1 and HTTP/2 support in the default binary.
- Experimental HTTP/3 support in a separate feature-enabled artifact.
- Local dashboard, metrics, admin CLI, benchmark runner, and release workflow.
- Linux x86_64 release artifacts and checksums.
- No hosted control plane and no required telemetry.

The main remaining adoption gap is distribution. A new user should not need a
Rust toolchain or repository checkout just to try kubio.

## User-Facing Target

The primary install path should be one command:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

Pinned and customized installs should also be possible:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_VERSION=v0.4.0 bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR=/usr/local/bin bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_FLAVOR=http3-experimental bash
```

After installation:

```bash
kubio --version
kubio update --check
kubio update
```

## Scope

In scope:

- Add a repository-root `install.sh` for Linux x86_64 installs.
- Keep v0.4.0 platform support explicit: Linux x86_64 only.
- Download release artifacts from GitHub Releases, not from a source build.
- Verify release checksums before installing.
- Install to a user-writable directory by default.
- Add CLI support for checking the latest release and updating the installed
  binary.
- Add best-effort update notices that do not affect proxy request handling.
- Add clear opt-out controls for automatic update checks.
- Refresh `README.md` so first-time users can understand what kubio does,
  install it, run it, update it, and find the right deeper docs without reading
  the design directory.
- Update release workflow artifacts, docs, and smoke tests around installation
  and update behavior.

Out of scope:

- macOS, Windows, Linux arm64, container package registries, Homebrew, apt, rpm,
  npm, or cargo-binstall support.
- A hosted update service.
- Automatic background updates.
- Runtime proxy behavior changes.
- Policy, cache, dashboard API, metrics, benchmark JSON, or config schema
  changes unrelated to install/update UX.
- Signed release attestations. Checksums are required in v0.4.0; stronger
  signing can be a later supply-chain release.

## Documents

- [PRD](PRD.md)
  - Product goals, user experience, non-goals, and success metrics.
- [Installer and Artifacts](installer-and-artifacts.md)
  - Shell installer contract, release asset naming, platform checks, checksum
    verification, and failure behavior.
- [Update Check and Self Update](update-check-and-self-update.md)
  - CLI commands, latest-version source, rate limiting, cache state, opt-out
    controls, and binary replacement rules.
- [Testing and Release](testing-release.md)
  - Unit, installer, update, CI, release workflow, and smoke gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone task breakdown.

## Cross-Cutting Constraints

- Distribution work must not weaken kubio's local-first model. Update checks
  are best-effort outbound GitHub release requests, never telemetry uploads.
- Installation and update paths must fail closed. If platform detection,
  download, checksum verification, or replacement fails, the existing binary is
  left untouched.
- Ambient update notices must not print to stdout. They may use stderr or
  tracing logs only, so command output remains scriptable.
- The request path must never wait on update checks.
- Unsupported platforms should fail with direct messages that name the detected
  OS/architecture and the supported target.
- The default install should not require root. Privileged installs are opt-in by
  choosing a privileged `KUBIO_INSTALL_DIR`.
- Every release artifact installed by v0.4.0 must be reproducible through the
  release workflow and covered by a checksum entry.

## Milestone Map

- M0: Design and command contract.
- M1: Release artifact contract and shell installer.
- M2: CLI update check and self update.
- M3: README, documentation, and onboarding.
- M4: CI and release workflow hardening.
- M5: Release notes and final compatibility audit.
