# Platform and Artifact Contract

Status: implemented locally; release workflow validation pending
Target release: `v0.4.1`

## 1. Supported Release Targets

v0.4.1 supports three release targets:

| Target | Host detection | Notes |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `uname -s=Linux`, `uname -m=x86_64` or `amd64` | Existing v0.4.0 target. |
| `aarch64-unknown-linux-gnu` | `uname -s=Linux`, `uname -m=aarch64` or `arm64` | New Linux arm64 target. |
| `aarch64-apple-darwin` | `uname -s=Darwin`, `uname -m=arm64` or `aarch64` | New Apple Silicon macOS target. |

Rejected examples:

- `Darwin:x86_64`;
- `Linux:armv7l`;
- `Linux:i686`;
- `MINGW`, `MSYS`, `CYGWIN`, or native Windows;
- empty or unknown `uname` output.

## 2. Artifact Names

Artifact names are derived from:

```text
kubio-<target>
kubio-<target>-http3-experimental
```

v0.4.1 releases should publish:

```text
kubio-x86_64-unknown-linux-gnu
kubio-x86_64-unknown-linux-gnu-http3-experimental
kubio-aarch64-unknown-linux-gnu
kubio-aarch64-unknown-linux-gnu-http3-experimental
kubio-aarch64-apple-darwin
kubio-aarch64-apple-darwin-http3-experimental
SHA256SUMS
install.sh
kubio-bench-h1.json
kubio-bench-h2.json
kubio-bench-h3.json
```

Benchmark JSON remains produced from the canonical Linux x86_64 benchmark gate.
It is not a per-platform performance claim.

## 3. Checksum Contract

`SHA256SUMS` must include exactly one entry for each binary artifact. The
installer and updater verify only the selected artifact but should tolerate a
checksum file that contains entries for all supported targets.

macOS uses `shasum -a 256` by default; Linux uses `sha256sum`. The installer
should abstract checksum verification behind a helper instead of assuming the
GNU command exists everywhere.

## 4. Install Manifest

The install manifest schema remains compatible with v0.4.0:

```json
{
  "schema_version": 1,
  "repo": "song-younghoon/kubio",
  "installed_path": "/home/user/.local/bin/kubio",
  "target": "aarch64-unknown-linux-gnu",
  "flavor": "standard",
  "installed_version": "0.4.1"
}
```

Rules:

- new installers write the detected target;
- new updaters read the target for diagnostics but choose the current host
  target for artifact selection;
- if the manifest target differs from current host target, `kubio update`
  should refuse by default and tell the user to reinstall;
- the manifest schema does not need to change unless new fields become
  necessary during implementation.

## 5. Compatibility

v0.4.1 must remain compatible with v0.4.0 installs:

- a v0.4.0 Linux x86_64 manifest can still be read;
- a v0.4.0 Linux x86_64 binary can update to v0.4.1;
- the standard and HTTP/3 flavor names stay unchanged;
- `KUBIO_VERSION`, `KUBIO_INSTALL_DIR`, `KUBIO_FLAVOR`, `KUBIO_REPO`, and
  `KUBIO_DOWNLOAD_BASE_URL` keep their v0.4.0 meanings.

## 6. Future-Proofing

Do not encode platform support as scattered string constants. Add a small
target model in both shell and Rust code:

- shell: `detect_target` and `artifact_name`;
- Rust: `ReleaseTarget::current()` and `Flavor::artifact_name(target)`.

That keeps future targets such as macOS x86_64 or Linux musl from requiring a
full updater rewrite.
