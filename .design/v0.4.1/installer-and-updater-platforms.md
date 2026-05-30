# Installer and Updater Platforms

Status: implemented; release workflow validated.
Target release: `v0.4.1`

## 1. Installer Platform Detection

`install.sh` should replace the v0.4.0 one-target case statement with a target
resolver:

```text
Linux:x86_64  -> x86_64-unknown-linux-gnu
Linux:amd64   -> x86_64-unknown-linux-gnu
Linux:aarch64 -> aarch64-unknown-linux-gnu
Linux:arm64   -> aarch64-unknown-linux-gnu
Darwin:arm64  -> aarch64-apple-darwin
Darwin:aarch64 -> aarch64-apple-darwin
```

Unsupported platforms fail before resolving download URLs.

The failure message should mention that v0.4.1 supports:

```text
x86_64 Linux, arm64 Linux, and arm64 macOS
```

## 2. Installer Required Tools

Common tools:

- `curl`;
- `uname`;
- `mktemp`;
- `chmod`;
- `sed`.

Checksum tool:

- prefer `sha256sum` when present;
- otherwise support `shasum -a 256`;
- fail clearly if neither exists.

macOS does not provide GNU `sha256sum` by default, so checksum verification must
not depend on it.

## 3. Installer Flow Changes

The high-level v0.4.0 flow stays the same:

1. Detect host target.
2. Resolve flavor.
3. Resolve artifact name.
4. Download artifact and `SHA256SUMS`.
5. Verify selected checksum.
6. Install as `kubio`.
7. Write install manifest.
8. Print version and `PATH` hint.

The only behavioral change is that target resolution can now return one of
three supported targets.

## 4. Updater Target Resolution

`kubio update` currently uses a Linux x86_64 target constant. v0.4.1 should
replace that with:

```rust
ReleaseTarget::current()
```

Suggested enum:

```rust
enum ReleaseTarget {
    X86_64UnknownLinuxGnu,
    Aarch64UnknownLinuxGnu,
    Aarch64AppleDarwin,
}
```

Responsibilities:

- derive from `std::env::consts::OS` and `std::env::consts::ARCH`;
- expose `triple()`;
- expose supported target diagnostics;
- fail before download for unsupported compiled hosts;
- feed `Flavor::artifact_name(target)`.

## 5. Manifest Interaction

The updater should read the install manifest as before. The manifest target is
not the source of truth for the artifact to download; the current host is.

Rules:

- if manifest target is missing or unknown, continue with current host target;
- if manifest target matches current host target, update normally;
- if manifest target differs from current host target, fail with a message that
  names both values and suggests reinstalling with `install.sh`;
- `--force` should not override cross-platform artifact mismatch by default.

This prevents a copied config file from making macOS download a Linux binary or
vice versa.

## 6. Flavor Interaction

Flavor selection remains:

1. explicit `--flavor`;
2. install manifest flavor;
3. compile-time `experimental-http3` feature;
4. `standard`.

Only artifact naming changes:

```text
standard: kubio-<target>
http3-experimental: kubio-<target>-http3-experimental
```

## 7. Replacement Behavior

Linux and macOS can use the same Unix replacement flow:

1. write a temp file in the target directory;
2. set executable mode;
3. move current binary to a unique backup path;
4. move the new binary into place;
5. run `<installed-path> --version`;
6. restore the backup on failure.

If macOS exposes an unexpected permission or quarantine issue during smoke
testing, the implementation should fail with a direct diagnostic rather than
attempting platform-specific workarounds in v0.4.1.

## 8. macOS Signing and Quarantine

v0.4.1 does not introduce code signing or notarization. The documented install
path is shell-based, so the binary is downloaded and installed by `curl` rather
than through a browser download flow.

Docs should state:

- artifacts are verified by `SHA256SUMS`;
- artifacts are not notarized packages;
- package-manager or notarized distribution is a future release track.
