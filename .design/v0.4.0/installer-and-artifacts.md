# Installer and Release Artifacts

Status: implemented scope
Target release: `v0.4.0`

## 1. Supported Platform

v0.4.0 supports one install target:

```text
x86_64-unknown-linux-gnu
```

Accepted host detection:

- `uname -s` must be `Linux`.
- `uname -m` may be `x86_64` or `amd64`.

Rejected examples:

- `aarch64` Linux.
- macOS on any architecture.
- Windows shells.
- Unknown or empty `uname` output.

Failure messages should include:

- detected OS;
- detected architecture;
- supported target;
- a note that v0.4.0 only ships Linux x86_64 binaries.

## 2. Release Assets

The release workflow should publish these assets for each tag:

```text
kubio-x86_64-unknown-linux-gnu
kubio-x86_64-unknown-linux-gnu-http3-experimental
SHA256SUMS
install.sh
kubio-bench-h1.json
kubio-bench-h2.json
kubio-bench-h3.json
```

Artifact names are part of the v0.4.0 install/update contract. Changing them
requires changing both the installer and `kubio update`.

## 3. Installer Location

Add a repository-root installer:

```text
install.sh
```

The primary documented command uses the main branch:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

The release workflow should also upload `install.sh` as a release asset so users
can choose a pinned installer source:

```bash
curl -fsSL https://github.com/song-younghoon/kubio/releases/download/v0.4.0/install.sh | bash
```

## 4. Installer Inputs

Supported environment variables:

| Variable | Default | Values |
| --- | --- | --- |
| `KUBIO_VERSION` | `latest` | `latest` or `vMAJOR.MINOR.PATCH` |
| `KUBIO_INSTALL_DIR` | `$HOME/.local/bin` or `/usr/local/bin` for root | absolute or relative directory path |
| `KUBIO_FLAVOR` | `standard` | `standard`, `http3-experimental` |
| `KUBIO_REPO` | `song-younghoon/kubio` | GitHub `owner/name`, mainly for tests and forks |
| `KUBIO_DOWNLOAD_BASE_URL` | derived from GitHub Releases | override for tests only |
| `KUBIO_FORCE` | unset | `1` permits overwriting an existing binary without version checks |

Unsupported values fail before download.

## 5. Download URLs

For latest stable installs:

```text
https://github.com/song-younghoon/kubio/releases/latest/download/<asset>
https://github.com/song-younghoon/kubio/releases/latest/download/SHA256SUMS
```

For pinned installs:

```text
https://github.com/song-younghoon/kubio/releases/download/<tag>/<asset>
https://github.com/song-younghoon/kubio/releases/download/<tag>/SHA256SUMS
```

The installer does not need to parse JSON for the common latest path.

## 6. Install Flow

The script should:

1. Enable strict shell behavior with `set -eu`.
2. Verify required tools: `curl`, `uname`, `mktemp`, `chmod`, `sha256sum`.
3. Detect OS and architecture.
4. Resolve artifact name from platform and flavor.
5. Resolve download base URL from version.
6. Create a temporary directory.
7. Download the artifact and `SHA256SUMS`.
8. Verify the selected artifact checksum.
9. Create the install directory if needed.
10. Copy or move the verified artifact into place as `kubio`.
11. Mark it executable.
12. Write an install manifest for self-update defaults.
13. Run `kubio --version`.
14. Print a `PATH` hint if the install directory is not on `PATH`.
15. Clean up temporary files.

The current binary must not be replaced until the downloaded artifact passes
checksum verification.

## 7. Install Manifest

The installer should write:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/kubio/install.json
```

Suggested fields:

```json
{
  "schema_version": 1,
  "repo": "song-younghoon/kubio",
  "installed_path": "/home/user/.local/bin/kubio",
  "target": "x86_64-unknown-linux-gnu",
  "flavor": "standard",
  "installed_version": "0.4.0"
}
```

If the manifest cannot be written, installation should still succeed after the
binary is installed, but the script should print a warning that `kubio update`
may need `--install-dir` or explicit path detection.

## 8. Failure Behavior

Failures should be short and actionable:

- missing tool: name the required tool;
- unsupported platform: name detected and supported platform;
- download failure: name the asset URL;
- checksum failure: say verification failed and leave existing binary untouched;
- unwritable install directory: name the directory and suggest setting
  `KUBIO_INSTALL_DIR`;
- existing binary conflict: overwrite only after verified download, or when the
  target path is the expected `kubio` path.

The installer should never call `sudo` itself.

## 9. Trust Model

v0.4.0 verifies that the downloaded binary matches `SHA256SUMS` from the same
GitHub Release. This protects against partial downloads and asset mismatches,
but it does not provide an independent signature. Stronger signing is a future
release candidate, likely with `cosign` or `minisign`.

Documentation should state this plainly.

## 10. Release Workflow Changes

The existing release workflow already builds the two Linux x86_64 binaries and
`SHA256SUMS`. v0.4.0 should add:

- upload `install.sh` as a release asset;
- run `bash -n install.sh`;
- run install smoke against locally staged `dist/` artifacts;
- verify the installed binary starts with `kubio --help`;
- verify `kubio --version` reports the tagged version;
- include installer usage in release notes.
