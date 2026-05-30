# Update Check and Self Update

Status: implemented scope
Target release: `v0.4.0`

## 1. Command Surface

Add a new top-level command:

```bash
kubio update --check
kubio update
kubio update --version v0.4.1
kubio update --flavor http3-experimental
kubio update --install-dir ~/.local/bin
```

Options:

| Option | Meaning |
| --- | --- |
| `--check` | Only check and print current/latest status. |
| `--version vX.Y.Z` | Install a pinned release instead of latest stable. |
| `--flavor standard|http3-experimental` | Override artifact flavor. |
| `--install-dir PATH` | Replace or install `PATH/kubio`. |
| `--force` | Allow updating even when the current executable looks like a development build. |

Add a global or command-level opt-out for ambient checks:

```bash
kubio serve --no-update-check --to http://localhost:3000
```

Environment opt-out:

```bash
KUBIO_UPDATE_CHECK=off
KUBIO_NO_UPDATE_CHECK=1
```

`KUBIO_UPDATE_CHECK=off` should be the documented form. `KUBIO_NO_UPDATE_CHECK=1`
is a compatibility-friendly alias.

## 2. Latest Release Source

Use GitHub Releases as the source of truth:

```text
GET https://api.github.com/repos/song-younghoon/kubio/releases/latest
```

Request behavior:

- send `User-Agent: kubio/<current-version>`;
- send `Accept: application/vnd.github+json`;
- use the cached `ETag` with `If-None-Match` when present;
- timeout after a short duration, targeted at 1500 ms;
- treat network failures as non-fatal for ambient checks;
- return a clear error for explicit `kubio update --check`.

The latest endpoint returns the latest non-prerelease GitHub Release, which
matches the default stable channel for v0.4.0.

## 3. Version Comparison

Supported release tags:

```text
vMAJOR.MINOR.PATCH
```

The parser should:

- accept a leading `v`;
- compare numeric major, minor, and patch parts;
- ignore malformed tags;
- treat pre-release tags as unsupported for v0.4.0 update checks.

A small internal parser is enough. A new `semver` dependency is optional but not
required for this narrow tag contract.

## 4. Update Check Cache

Use:

```text
${XDG_CACHE_HOME:-$HOME/.cache}/kubio/update-check.json
```

Suggested fields:

```json
{
  "schema_version": 1,
  "checked_at_unix": 1710000000,
  "latest_version": "0.4.1",
  "latest_url": "https://github.com/song-younghoon/kubio/releases/tag/v0.4.1",
  "etag": "\"abc123\""
}
```

Default ambient check interval:

```text
24 hours
```

Explicit commands ignore the interval:

- `kubio update --check`
- `kubio update`

If the cache cannot be read or written, checks should still work and emit no
ambient warning unless the explicit command needs to explain the failure.

## 5. Ambient Notices

Ambient update notices should be intentionally narrow:

- `kubio serve` checks in the background after listeners are configured.
- `kubio doctor` may check after its normal diagnostics.
- `routes`, `explain`, and `purge` should not emit ambient notices because they
  are more likely to be scripted.

All ambient notices go to stderr or tracing logs, never stdout.

Example notice:

```text
kubio 0.4.1 is available; current is 0.4.0. Run `kubio update`.
```

No request handling task should await this network call.

## 6. Self-Update Artifact Selection

Artifact name is derived from:

- platform: Linux x86_64 only;
- flavor: installer manifest, compile-time feature, or explicit `--flavor`;
- version: latest stable or explicit `--version`.

Flavor defaults:

1. `--flavor` when supplied.
2. `install.json` flavor when present.
3. `http3-experimental` when the current binary was compiled with the
   `experimental-http3` feature.
4. `standard`.

## 7. Self-Update Flow

`kubio update` should:

1. Determine target version.
2. Determine install path from `--install-dir`, install manifest, or current
   executable.
3. Refuse to update paths under `target/debug` or `target/release` unless
   `--force` is set.
4. Determine artifact name.
5. Download artifact and `SHA256SUMS` into a temporary directory.
6. Verify checksum.
7. Mark downloaded artifact executable.
8. Replace the existing binary atomically when possible.
9. Update `install.json`.
10. Print the installed version.

If the install path is not writable, fail with a direct message. The command
should not invoke `sudo`.

## 8. Replacement Rules

Preferred replacement on Linux:

1. Create a temp file in the same directory as the target binary.
2. Write and verify the new binary before replacement.
3. Rename the existing binary to `kubio.old` or a unique backup name.
4. Rename the new binary to `kubio`.
5. Run `kubio --version` from the installed path.
6. Remove backup after success, or keep it if final verification fails.

If atomic rename is not available, fail rather than doing a partial overwrite.

Running Linux executables can be renamed while the current process continues, so
self-update can replace the file used to start the running process.

## 9. Output Contract

Explicit update check:

```text
kubio 0.4.0 is current.
```

```text
kubio 0.4.1 is available. Run `kubio update` to install it.
```

Successful update:

```text
Updated kubio from 0.4.0 to 0.4.1 at /home/user/.local/bin/kubio.
```

Already current:

```text
kubio 0.4.0 is already current.
```

Network errors from `kubio update --check` should return a non-zero exit code
and a message naming the failed endpoint. Ambient checks should only log at
debug level or suppress the error.

## 10. Privacy and Local-First Notes

Update checks contact GitHub only for public release metadata and assets. They
must not include:

- origin URLs;
- route paths;
- cache keys;
- request headers;
- dashboard data;
- config contents;
- hostnames beyond the normal HTTP request destination.

Documentation should include the opt-out environment variables.
