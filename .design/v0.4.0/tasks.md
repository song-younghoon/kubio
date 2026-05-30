# v0.4.0 Implementation Tasks

Status: implemented scope
Target release: `v0.4.0`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.4.0 scope

## Current Implementation Snapshot

kubio has a repository-root installer, Linux x86_64 release artifacts,
checksums, a user-facing update check, and a verified self-update command.

## M0: Design and Command Contract

Goal: lock the distribution scope and user-facing commands before
implementation.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.4.0 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add installer and release artifact design.
- [x] M0.1.4 Add update check and self-update design.
- [x] M0.1.5 Add testing and release plan.
- [x] M0.1.6 Add implementation task breakdown.

Acceptance:

- Scope clearly prioritizes distribution and usability over proxy features.
- Linux x86_64 is the only supported v0.4.0 install target.
- `kubio update --check` and `kubio update` are the agreed command names.

## M1: Release Artifact Contract and Installer

Goal: make release artifacts installable without a Rust toolchain.

### M1.1 Artifact Contract

- [x] M1.1.1 Confirm standard artifact name:
  `kubio-x86_64-unknown-linux-gnu`.
- [x] M1.1.2 Confirm HTTP/3 artifact name:
  `kubio-x86_64-unknown-linux-gnu-http3-experimental`.
- [x] M1.1.3 Confirm `SHA256SUMS` includes both binaries.
- [x] M1.1.4 Add `install.sh` to release assets.
- [x] M1.1.5 Keep benchmark JSON artifacts in the release workflow.

Acceptance:

- Installer and updater can derive artifact URLs deterministically.
- Release notes list all install-relevant assets.

### M1.2 Shell Installer

- [x] M1.2.1 Add repository-root `install.sh`.
- [x] M1.2.2 Add strict shell mode and cleanup trap.
- [x] M1.2.3 Check required tools.
- [x] M1.2.4 Detect Linux x86_64 and fail other platforms clearly.
- [x] M1.2.5 Support `KUBIO_VERSION`.
- [x] M1.2.6 Support `KUBIO_INSTALL_DIR`.
- [x] M1.2.7 Support `KUBIO_FLAVOR`.
- [x] M1.2.8 Support test-only `KUBIO_REPO` and `KUBIO_DOWNLOAD_BASE_URL`.
- [x] M1.2.9 Download artifact and `SHA256SUMS`.
- [x] M1.2.10 Verify selected artifact checksum.
- [x] M1.2.11 Install as `kubio`.
- [x] M1.2.12 Print `PATH` hint when needed.
- [x] M1.2.13 Write install manifest when possible.

Acceptance:

- A clean Linux x86_64 host can install with the documented curl command.
- Checksum mismatch leaves any existing binary untouched.

## M2: CLI Update Check and Self Update

Goal: let installed binaries discover and install newer stable releases.

### M2.1 CLI Argument Surface

- [x] M2.1.1 Add `UpdateArgs`.
- [x] M2.1.2 Add `Command::Update`.
- [x] M2.1.3 Add `commands::update`.
- [x] M2.1.4 Add `--check`, `--version`, `--flavor`, `--install-dir`, and
  `--force`.
- [x] M2.1.5 Add `--no-update-check` where ambient checks are supported.
- [x] M2.1.6 Add environment opt-outs:
  `KUBIO_UPDATE_CHECK=off` and `KUBIO_NO_UPDATE_CHECK=1`.

Acceptance:

- `kubio update --help` documents all update options.
- Existing commands keep their current output unless ambient checks are enabled
  for that command and write only to stderr/logs.

### M2.2 Release Metadata Client

- [x] M2.2.1 Add latest-release client using GitHub Releases API.
- [x] M2.2.2 Set user agent and accept headers.
- [x] M2.2.3 Add timeout.
- [x] M2.2.4 Parse stable `vMAJOR.MINOR.PATCH` tags.
- [x] M2.2.5 Compare current and latest versions.
- [x] M2.2.6 Add update-check cache under XDG cache directory.
- [x] M2.2.7 Store and send `ETag`.
- [x] M2.2.8 Treat ambient network failures as non-fatal.

Acceptance:

- Explicit check reports current, newer, and unreachable states.
- Ambient check never blocks proxy request handling.

### M2.3 Self Update

- [x] M2.3.1 Resolve install path from `--install-dir`, manifest, or current
  executable.
- [x] M2.3.2 Refuse development paths under `target/debug` or `target/release`
  unless `--force` is set.
- [x] M2.3.3 Resolve flavor from CLI, manifest, build feature, or default.
- [x] M2.3.4 Download artifact and `SHA256SUMS`.
- [x] M2.3.5 Verify checksum.
- [x] M2.3.6 Replace binary atomically where possible.
- [x] M2.3.7 Keep or restore previous binary on failure.
- [x] M2.3.8 Update install manifest after success.

Acceptance:

- `kubio update` can update a staged test install.
- Failure after download leaves the old binary runnable.

## M3: README, Documentation, and Onboarding

Goal: make install/update behavior easy to find and honest about limits, with
`README.md` serving as a friendly first-run path for new users.

- [x] M3.1 Rewrite README opening sections for first-time users:
  what kubio is, when to use it, safety default, and current release status.
- [x] M3.2 Update README Quick Start to lead with one-command install.
- [x] M3.3 Add README install variants for pinned version, custom install dir,
  and HTTP/3 experimental flavor.
- [x] M3.4 Add README update section covering `kubio update --check`,
  `kubio update`, and opt-out controls.
- [x] M3.5 Keep source-build and development commands in README, but make them
  secondary to released-binary installation.
- [x] M3.6 Add README links to getting started, deployment, configuration,
  metrics, safety model, and release notes.
- [x] M3.7 Update `docs/getting-started.md`.
- [x] M3.8 Update `docs/deployment.md`.
- [x] M3.9 Add install/update environment variable reference.
- [x] M3.10 Document Linux x86_64-only support.
- [x] M3.11 Document checksum trust model.
- [x] M3.12 Document update-check opt-out controls.
- [x] M3.13 Add `docs/release-notes-v0.4.0.md`.

Acceptance:

- A user can install, run `kubio --version`, check for updates, and update using
  documented commands only.
- README does not require a reader to know Cargo, GitHub Releases, or the design
  directory before trying kubio.

## M4: CI and Release Workflow Hardening

Goal: prove install and update work from staged release artifacts.

- [x] M4.1 Run `bash -n install.sh` in CI.
- [x] M4.2 Add local staged install smoke for standard artifact.
- [x] M4.3 Add local staged install smoke for HTTP/3 experimental artifact.
- [x] M4.4 Add checksum mismatch installer smoke.
- [x] M4.5 Add update-check client tests with fixtures.
- [x] M4.6 Add self-update integration smoke with staged artifacts.
- [x] M4.7 Upload `install.sh` as a release asset.
- [x] M4.8 Verify release asset list before publishing.

Acceptance:

- Release workflow fails if installer or update smoke fails.
- Tagged release contains binaries, checksums, installer, and benchmark JSON.

## M5: Release Hardening

Goal: ship v0.4.0 as a packaging and usability release.

- [x] M5.1 Bump workspace version to `0.4.0`.
- [x] M5.2 Confirm `kubio --version` reports `0.4.0`.
- [x] M5.3 Run full default release gate.
- [x] M5.4 Run HTTP/3 feature release gate.
- [x] M5.5 Run installer smoke from staged artifacts.
- [x] M5.6 Run update smoke from staged artifacts.
- [x] M5.7 Confirm ambient notices do not print to stdout.
- [x] M5.8 Confirm unsupported platforms fail before download.
- [x] M5.9 Confirm release notes and docs mention platform limits.

Acceptance:

- v0.4.0 can be installed and updated on Linux x86_64 with documented commands.
- No new proxy behavior is advertised as part of the release.
