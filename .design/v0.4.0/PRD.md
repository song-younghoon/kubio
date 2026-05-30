# PRD: kubio v0.4.0

Document status: implemented scope
Target release: `v0.4.0`
Core philosophy: **make the existing product easy to get and keep current**

## 1. Product Summary

kubio v0.4.0 is a distribution and usability release. It should make a released
kubio binary installable with a single shell command, make newer releases
visible to operators, and provide a safe self-update path.

The release should not add new proxy capabilities. The product value is that a
user can install and maintain the existing kubio runtime without cloning the
repository or installing Rust.

## 2. Background

Earlier releases focused on proxy safety, revalidation, disk storage,
protocol support, benchmarks, and maintainable source layout. The release
workflow already builds Linux x86_64 binaries and checksums, but the user
experience still assumes a user knows where to find an artifact and how to place
it on `PATH`.

For v0.4.0, kubio should feel like a small CLI tool:

- one command installs the latest stable release;
- unsupported platforms fail clearly;
- installed binaries can check whether a newer release exists;
- updating reuses the same verified release artifact path as installation.

## 3. Goals

v0.4.0 should:

1. Add a repository-root `install.sh` that installs kubio from GitHub Releases.
2. Support Linux x86_64 only, with explicit detection and failure messages for
   other platforms.
3. Default to the standard release artifact and allow explicit installation of
   the HTTP/3 experimental artifact.
4. Verify `SHA256SUMS` before installing.
5. Install to `$HOME/.local/bin` by default when not running as root.
6. Allow `KUBIO_INSTALL_DIR` to override the target directory.
7. Allow `KUBIO_VERSION=vX.Y.Z` to install a pinned release.
8. Add `kubio update --check` to report whether a newer stable release exists.
9. Add `kubio update` to download, verify, and replace the installed binary.
10. Rate-limit automatic update checks and make them easy to disable.
11. Keep automatic update notices out of stdout and out of the request path.
12. Rewrite `README.md` around the released-binary path so a first-time user can
    understand kubio, install it, run it, update it, and find next-step docs.
13. Document the trust model, supported platform, environment variables, and
    rollback behavior.
14. Add CI and release gates that smoke-test installation and update logic.

## 4. User Experience

### 4.1 Install Latest Stable

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

Expected behavior:

- The script detects Linux x86_64.
- The script downloads the latest stable standard artifact and `SHA256SUMS`.
- The checksum for the selected artifact is verified.
- The binary is installed as `kubio` under `$HOME/.local/bin` by default.
- The script prints the installed version and a `PATH` hint if needed.

### 4.2 Install a Pinned Version

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_VERSION=v0.4.0 bash
```

Expected behavior:

- The script downloads from the pinned release URL.
- If the release or artifact does not exist, installation fails before touching
  the current binary.

### 4.3 Install to a Specific Directory

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR=/usr/local/bin bash
```

Expected behavior:

- The script uses the requested directory.
- If the directory is not writable, the script fails with a message suggesting a
  user-writable directory or rerunning with appropriate privileges.
- The script should not invoke `sudo` itself.

### 4.4 Install the HTTP/3 Experimental Artifact

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_FLAVOR=http3-experimental bash
```

Expected behavior:

- The script installs `kubio-x86_64-unknown-linux-gnu-http3-experimental`.
- The installed binary is named `kubio`.
- The installer records the selected flavor for future `kubio update` defaults.

### 4.5 Check for Updates

```bash
kubio update --check
```

Example output when current:

```text
kubio 0.4.0 is current.
```

Example output when newer:

```text
kubio 0.4.1 is available. Run `kubio update` to install it.
```

### 4.6 Update

```bash
kubio update
```

Expected behavior:

- The command resolves the latest stable GitHub Release.
- The selected artifact and `SHA256SUMS` are downloaded.
- The checksum is verified.
- The current installed binary is replaced atomically when possible.
- If replacement fails, the previous binary remains runnable.

## 5. Non-Goals

v0.4.0 will not:

- Build kubio from source during installation.
- Add package manager integrations.
- Add a daemon or background updater.
- Auto-update without an explicit `kubio update` command.
- Support platforms beyond Linux x86_64.
- Add a kubio-hosted release metadata service.
- Change cache policy, proxy modes, storage behavior, dashboard APIs, metrics,
  or benchmark semantics.
- Guarantee update checks work in offline, air-gapped, or GitHub-blocked
  environments.

## 6. Product Principles

### 6.1 Install From Releases, Not From Main

The install script may live on `main`, but binaries come from immutable GitHub
Release assets. The script must not compile or download source archives.

### 6.2 Verify Before Touching the Current Binary

Checksum verification happens before copying or replacing the installed binary.
Failure should leave the current installation unchanged.

### 6.3 Be Honest About Platform Support

The only supported target for v0.4.0 is Linux x86_64. Other platforms should not
attempt a partial install or suggest that support is present.

### 6.4 Do Not Break Scriptability

Command output used by scripts should stay stable. Ambient update notices go to
stderr or logs, not stdout, and explicit update commands own their own output.

### 6.5 Keep Local-First Trust Boundaries Clear

Update checks request public GitHub release metadata. kubio does not send route,
origin, cache, dashboard, config, hostname, or workload data.

## 7. Success Metrics

The release is successful when:

- The README leads with the one-command install path, states Linux x86_64-only
  support clearly, and keeps source-build instructions secondary.
- A clean Linux x86_64 host can install kubio with the documented curl command.
- The installer works without Rust, git, or a repository checkout.
- The installer refuses unsupported platforms with clear messages.
- Checksum verification is required for both install and update.
- `kubio update --check` distinguishes current, newer, and unreachable states.
- `kubio update` can update a v0.4.0 install to a later test release artifact in
  release workflow smoke tests.
- Automatic update checks are rate-limited and opt-out.
- Existing proxy tests, benchmark gates, dashboard routes, metrics, and config
  parsing continue to pass.
