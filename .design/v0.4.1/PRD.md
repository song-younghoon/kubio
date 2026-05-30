# PRD: kubio v0.4.1

Document status: implemented locally; release workflow validation pending
Target release: `v0.4.1`
Core philosophy: **make the v0.4 installer work on the common arm64 hosts**

## 1. Product Summary

kubio v0.4.1 is a distribution follow-up release. It should extend the
v0.4.0 install/update experience from Linux x86_64 to Linux arm64 and Apple
Silicon macOS while keeping the same one-command install path.

This is not a proxy feature release. Users should see the same runtime behavior,
CLI commands, safety defaults, and update model. The visible difference is that
more machines can install and update released binaries without a Rust toolchain.

## 2. Background

v0.4.0 made kubio installable from GitHub Releases, but intentionally limited
support to `x86_64-unknown-linux-gnu`. That was the right first release shape:
one platform, deterministic artifacts, checksum verification, and a working
self-update path.

The repository now has access to an arm64 macOS self-hosted runner. That makes
native Apple Silicon build and smoke testing practical. Linux arm64 artifacts
can be built with a GNU aarch64 cross toolchain on GitHub-hosted Linux, with
QEMU limited to short binary execution smoke checks instead of a slow emulated
Rust build.

## 3. Goals

v0.4.1 should:

1. Keep the existing install command unchanged.
2. Add release support for `aarch64-unknown-linux-gnu`.
3. Add release support for `aarch64-apple-darwin`.
4. Continue supporting `x86_64-unknown-linux-gnu`.
5. Publish standard and HTTP/3 experimental artifacts for every supported
   release target.
6. Keep a single `SHA256SUMS` file that covers every release binary.
7. Update `install.sh` to detect Linux x86_64, Linux arm64, and macOS arm64.
8. Update `kubio update` so artifact selection uses the current host target.
9. Keep install and update checksum verification mandatory.
10. Refactor the release workflow into platform build jobs plus a final publish
    job.
11. Use the self-hosted arm64 macOS runner for native macOS builds.
12. Add release gates that prove each supported target has both flavor artifacts
    and checksum entries.
13. Update docs so the support matrix is easy to find and does not overpromise
    package-manager or notarized macOS distribution.

## 4. User Experience

### 4.1 Install on a Supported Host

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

Expected behavior:

- Linux x86_64 selects `kubio-x86_64-unknown-linux-gnu`.
- Linux arm64 selects `kubio-aarch64-unknown-linux-gnu`.
- macOS arm64 selects `kubio-aarch64-apple-darwin`.
- The artifact is verified with `SHA256SUMS`.
- The binary is installed as `kubio`.
- The install manifest records the selected target and flavor.

### 4.2 Install the HTTP/3 Experimental Flavor

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_FLAVOR=http3-experimental bash
```

Expected behavior:

- The host target is detected normally.
- The `-http3-experimental` artifact for that target is selected.
- The installed binary is still named `kubio`.

### 4.3 Unsupported Host

On macOS x86_64, Windows, or unsupported Linux architectures, the installer
should fail before download with a message that includes:

- detected OS;
- detected architecture;
- supported targets.

### 4.4 Self Update

```bash
kubio update
```

Expected behavior:

- The updater detects the current host target.
- The updater chooses the matching release artifact and flavor.
- If the install manifest target conflicts with the current host target, the
  updater refuses to proceed rather than installing the manifest target blindly.
- The replacement and rollback behavior remains the same as v0.4.0.

## 5. Non-Goals

v0.4.1 will not:

- Add new proxy behavior.
- Add Windows support.
- Add macOS x86_64 support.
- Add musl Linux support.
- Add Homebrew, apt, rpm, npm, or cargo-binstall distribution.
- Add signed or notarized macOS packages.
- Add automatic background updates.
- Publish a multi-architecture Docker image unless separately scoped.
- Replace GitHub Releases as the source of install/update artifacts.

## 6. Product Principles

### 6.1 Preserve the v0.4 UX

The same command should install kubio on every supported platform. Platform
selection is automatic and deterministic.

### 6.2 Support Means Build Plus Smoke

A target is not supported merely because `cargo build --target` succeeds. Every
supported target needs at least a release artifact, checksum entry, installer
selection test, updater artifact selection test, and basic executable smoke.

### 6.3 Fail Before Download on Unsupported Hosts

Unsupported hosts should get clear messages without making network requests for
artifacts that cannot work.

### 6.4 Keep Publishing Centralized

Self-hosted macOS runners should build and upload workflow artifacts, but they
should not publish the GitHub Release. A final publish job should aggregate all
platform artifacts, compute `SHA256SUMS`, verify the asset list, and publish.

### 6.5 Be Honest About macOS Trust Boundaries

v0.4.1 macOS support covers shell-installed CLI binaries verified by
`SHA256SUMS`. It does not imply notarized packages or Gatekeeper-friendly app
distribution for browser-downloaded binaries.

## 7. Success Metrics

The release is successful when:

- `README.md` and `docs/install-update.md` list all supported targets.
- The one-command installer succeeds on Linux x86_64, Linux arm64, and macOS
  arm64 test environments.
- The installer rejects unsupported hosts before artifact download.
- `kubio update` derives the correct target artifact on each supported host.
- The release workflow publishes all expected standard and HTTP/3 experimental
  artifacts.
- `SHA256SUMS` contains every published binary exactly once.
- The macOS arm64 self-hosted runner performs native macOS `--version`,
  `--help`, and staged install/update smoke checks.
- Linux arm64 artifacts are cross-built with the GNU aarch64 toolchain and
  smoke-checked with short QEMU `--version` and `--help` runs.
- Existing Linux x86_64 tests, benchmark gates, and release smoke remain green.
