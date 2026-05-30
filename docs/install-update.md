# Install and Update

kubio v0.4.0 installs released binaries from GitHub Releases. The installer and
self-update command support Linux x86_64.

## One-Command Install

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

The installer:

- detects Linux x86_64;
- downloads the selected release artifact and `SHA256SUMS`;
- verifies the artifact checksum;
- installs the binary as `kubio`;
- writes an install manifest when possible.

The default install directory is `$HOME/.local/bin` for normal users and
`/usr/local/bin` for root.

## Installer Variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `KUBIO_VERSION` | `latest` | Install `latest` or a pinned tag such as `v0.4.0`. |
| `KUBIO_INSTALL_DIR` | `$HOME/.local/bin` | Install `kubio` into a specific directory. |
| `KUBIO_FLAVOR` | `standard` | Use `standard` or `http3-experimental`. |
| `KUBIO_REPO` | `song-younghoon/kubio` | Override the GitHub repository for forks. |
| `KUBIO_DOWNLOAD_BASE_URL` | GitHub Releases URL | Override artifact downloads for tests or mirrors. |

Examples:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_VERSION=v0.4.0 bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR=/usr/local/bin bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_FLAVOR=http3-experimental bash
```

## Update Checks

```bash
kubio update --check
```

When a newer stable release exists, kubio prints:

```text
kubio 0.4.1 is available. Run `kubio update` to install it.
```

When current:

```text
kubio 0.4.0 is current.
```

## Self Update

```bash
kubio update
```

The updater downloads the release artifact and `SHA256SUMS`, verifies the
checksum, writes a new executable next to the current install path, and replaces
the binary after verification. If verification or replacement fails, kubio tries
to keep the previous binary in place.

Useful options:

```bash
kubio update --version v0.4.1
kubio update --flavor http3-experimental
kubio update --install-dir ~/.local/bin
```

The updater refuses to replace development binaries under `target/debug` or
`target/release` unless `--force` is passed.

## Ambient Notice Opt-Out

`kubio serve` performs a best-effort, rate-limited update check after startup.
It never blocks request handling and never writes notices to stdout.

Disable ambient checks:

```bash
KUBIO_UPDATE_CHECK=off kubio serve --to http://localhost:3000
KUBIO_NO_UPDATE_CHECK=1 kubio serve --to http://localhost:3000
kubio serve --no-update-check --to http://localhost:3000
```

## Trust Model

kubio v0.4.0 verifies that the downloaded artifact matches `SHA256SUMS` from the
same GitHub Release. This catches partial downloads and asset mismatches, but it
is not an independent signature. Stronger signed provenance is a future
supply-chain hardening item.
