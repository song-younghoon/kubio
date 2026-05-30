# v0.4.1 Implementation Tasks

Status: implemented locally; release workflow validation pending
Target release: `v0.4.1`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.4.1 scope

## Current Implementation Snapshot

kubio v0.4.0 installs and updates Linux x86_64 release binaries. v0.4.1 should
extend that distribution path to Linux arm64 and macOS arm64 while preserving
the same install/update UX.

## M0: Design and Platform Contract

Goal: lock the release matrix and artifact names before implementation.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.4.1 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add platform and artifact contract.
- [x] M0.1.4 Add installer and updater platform design.
- [x] M0.1.5 Add release workflow design.
- [x] M0.1.6 Add testing and release plan.
- [x] M0.1.7 Add implementation task breakdown.

Acceptance:

- Scope clearly stays focused on release platform coverage.
- Supported targets are `x86_64-unknown-linux-gnu`,
  `aarch64-unknown-linux-gnu`, and `aarch64-apple-darwin`.
- Artifact names are deterministic for standard and HTTP/3 experimental flavors.

## M1: Installer Platform Expansion

Goal: make `install.sh` choose the right artifact on supported hosts.

- [x] M1.1 Add a shell target resolver for Linux x86_64, Linux arm64, and macOS
  arm64.
- [x] M1.2 Update unsupported-platform messages to list all v0.4.1 targets.
- [x] M1.3 Add portable checksum verification using `sha256sum` or
  `shasum -a 256`.
- [x] M1.4 Keep `KUBIO_VERSION`, `KUBIO_INSTALL_DIR`, `KUBIO_FLAVOR`,
  `KUBIO_REPO`, and `KUBIO_DOWNLOAD_BASE_URL` behavior compatible.
- [x] M1.5 Write the detected target into the install manifest.
- [x] M1.6 Add staged installer smoke for macOS arm64.
- [x] M1.7 Add staged installer target-selection tests for Linux arm64.

Acceptance:

- A supported host installs without specifying a platform.
- Unsupported hosts fail before download.
- macOS installs do not require GNU `sha256sum`.

## M2: Updater Platform Expansion

Goal: make `kubio update` select the artifact for the current host.

- [x] M2.1 Replace the Linux x86_64 target constant with a `ReleaseTarget`
  model.
- [x] M2.2 Add current-host target detection for Linux x86_64, Linux arm64, and
  macOS arm64.
- [x] M2.3 Change `Flavor::artifact_name()` to accept a release target.
- [x] M2.4 Refuse update when manifest target conflicts with current host
  target.
- [x] M2.5 Keep v0.4.0 Linux x86_64 manifest compatibility.
- [x] M2.6 Add unit tests for target mapping and artifact names.
- [x] M2.7 Add staged self-update smoke on macOS arm64.

Acceptance:

- `kubio update` never downloads a different-platform artifact for a supported
  host.
- Linux x86_64 users can update from v0.4.0 to v0.4.1.

## M3: Release Workflow Refactor

Goal: publish all platform artifacts from one final release job.

- [x] M3.1 Split Linux x86_64 build from release publishing.
- [x] M3.2 Add Linux arm64 GNU cross-build job.
- [x] M3.3 Add macOS arm64 self-hosted runner job.
- [x] M3.4 Add macOS runner preflight for `Darwin arm64`.
- [x] M3.5 Upload platform binaries as workflow artifacts.
- [x] M3.6 Add final `publish-release` job that downloads all platform
  artifacts.
- [x] M3.7 Compute one `SHA256SUMS` file over all binary artifacts.
- [x] M3.8 Verify the expected release asset list before publishing.
- [x] M3.9 Keep Docker image smoke independent from platform binary publishing.
- [x] M3.10 Add release workflow concurrency.

Acceptance:

- Only the final publish job has `contents: write`.
- A tag push publishes six binary assets, one checksum file, installer, and
  benchmark JSON.

## M4: Platform Smoke and Release Gates

Goal: prove every supported release target is usable enough to advertise.

- [x] M4.1 Keep existing Linux x86_64 full release gate.
- [x] M4.2 Run `--help` and `--version` for Linux x86_64 standard and HTTP/3
  artifacts.
- [x] M4.3 Validate Linux arm64 artifacts are ELF aarch64 binaries.
- [x] M4.4 Run Linux arm64 `--help` and `--version` through short
  `qemu-aarch64` smoke.
- [x] M4.5 Run native macOS arm64 `--help` and `--version` for both flavors.
- [x] M4.6 Run native macOS arm64 staged install smoke.
- [x] M4.7 Run native macOS arm64 staged self-update smoke.
- [x] M4.8 Verify checksum mismatch still leaves the existing binary untouched.

Acceptance:

- Release workflow fails if any supported target artifact is missing or has no
  checksum.
- macOS arm64 support is backed by native execution on the self-hosted runner.
- Linux arm64 support is backed by GNU cross-builds plus short QEMU startup
  smoke. Full native install/update smoke is deferred until a native Linux
  arm64 runner is available.

## M5: README, Documentation, and Release Notes

Goal: make platform support easy to understand and honest about limits.

- [x] M5.1 Update `README.md` install section from Linux x86_64-only to the
  v0.4.1 support matrix.
- [x] M5.2 Update `docs/install-update.md`.
- [x] M5.3 Update `docs/getting-started.md`.
- [x] M5.4 Update `docs/deployment.md`.
- [x] M5.5 Add `docs/release-notes-v0.4.1.md`.
- [x] M5.6 Update `docs/roadmap.md`.
- [x] M5.7 Document unsupported macOS x86_64 and Windows.
- [x] M5.8 Document macOS artifacts as checksum-verified CLI binaries, not
  notarized packages.

Acceptance:

- A user can tell whether their host is supported before running the installer.
- Docs do not imply package-manager, notarization, or Windows support.

## M6: Release Hardening

Goal: ship v0.4.1 as a platform-coverage patch release.

- [x] M6.1 Bump workspace version to `0.4.1`.
- [x] M6.2 Confirm `kubio --version` reports `0.4.1`.
- [x] M6.3 Run full Linux x86_64 release gate.
- [x] M6.4 Run Linux arm64 cross-build and startup smoke gate.
- [x] M6.5 Run macOS arm64 native build and smoke gate.
- [ ] M6.6 Confirm release assets and `SHA256SUMS` after publish.
- [x] M6.7 Confirm a v0.4.0 Linux x86_64 install can update to v0.4.1.

Acceptance:

- v0.4.1 can be installed and updated on Linux x86_64, Linux arm64, and macOS
  arm64 with documented commands.
- No new proxy behavior is advertised as part of the release.
