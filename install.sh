#!/usr/bin/env bash
set -euo pipefail

repo="${KUBIO_REPO:-song-younghoon/kubio}"
version="${KUBIO_VERSION:-latest}"
flavor="${KUBIO_FLAVOR:-standard}"

fail() {
  printf 'kubio install: %s\n' "$*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

need curl
need uname
need mktemp
need chmod
need sed

os="$(uname -s 2>/dev/null || true)"
arch="$(uname -m 2>/dev/null || true)"

case "$os:$arch" in
  Linux:x86_64|Linux:amd64)
    target="x86_64-unknown-linux-gnu"
    ;;
  Linux:aarch64|Linux:arm64)
    target="aarch64-unknown-linux-gnu"
    ;;
  Darwin:arm64|Darwin:aarch64)
    target="aarch64-apple-darwin"
    ;;
  *)
    fail "unsupported platform: os=${os:-unknown} arch=${arch:-unknown}; v0.4.1 supports Linux x86_64, Linux arm64, and macOS arm64 only"
    ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
  checksum_tool="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  checksum_tool="shasum"
else
  fail "required command not found: sha256sum or shasum"
fi

case "$flavor" in
  standard)
    artifact="kubio-${target}"
    ;;
  http3-experimental)
    artifact="kubio-${target}-http3-experimental"
    ;;
  *)
    fail "unsupported KUBIO_FLAVOR=${flavor}; expected standard or http3-experimental"
    ;;
esac

if [ "$version" != "latest" ] && ! [[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  fail "unsupported KUBIO_VERSION=${version}; expected latest or vMAJOR.MINOR.PATCH"
fi

if [ "${KUBIO_INSTALL_DIR:-}" ]; then
  install_dir="$KUBIO_INSTALL_DIR"
elif [ "${EUID:-$(id -u)}" -eq 0 ]; then
  install_dir="/usr/local/bin"
else
  install_dir="${HOME:?HOME is required}/.local/bin"
fi

if [ "${KUBIO_DOWNLOAD_BASE_URL:-}" ]; then
  base_url="${KUBIO_DOWNLOAD_BASE_URL%/}"
elif [ "$version" = "latest" ]; then
  base_url="https://github.com/${repo}/releases/latest/download"
else
  base_url="https://github.com/${repo}/releases/download/${version}"
fi

tmpdir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT INT TERM

printf 'Installing kubio (%s, %s) from %s\n' "$target" "$flavor" "$base_url"

curl -fsSL "${base_url}/${artifact}" -o "${tmpdir}/${artifact}" \
  || fail "failed to download ${base_url}/${artifact}"
curl -fsSL "${base_url}/SHA256SUMS" -o "${tmpdir}/SHA256SUMS" \
  || fail "failed to download ${base_url}/SHA256SUMS"

checksum_line="$(grep -E "[[:space:]]${artifact}\$" "${tmpdir}/SHA256SUMS" || true)"
if [ -z "$checksum_line" ]; then
  fail "SHA256SUMS does not contain ${artifact}"
fi

if [ "$checksum_tool" = "sha256sum" ]; then
  (cd "$tmpdir" && printf '%s\n' "$checksum_line" | sha256sum -c - >/dev/null) \
    || fail "checksum verification failed for ${artifact}; existing kubio was not changed"
else
  set -- $checksum_line
  expected_checksum="$1"
  actual_checksum_line="$(shasum -a 256 "${tmpdir}/${artifact}")" \
    || fail "checksum verification failed for ${artifact}; existing kubio was not changed"
  set -- $actual_checksum_line
  actual_checksum="$1"
  if [ "$expected_checksum" != "$actual_checksum" ]; then
    fail "checksum verification failed for ${artifact}; existing kubio was not changed"
  fi
fi

mkdir -p "$install_dir" || fail "failed to create install directory: ${install_dir}"
if [ ! -w "$install_dir" ]; then
  fail "install directory is not writable: ${install_dir}; set KUBIO_INSTALL_DIR to a writable directory"
fi

install_path="${install_dir%/}/kubio"
chmod 0755 "${tmpdir}/${artifact}"
cp "${tmpdir}/${artifact}" "$install_path" \
  || fail "failed to install kubio to ${install_path}"

version_output="$("$install_path" --version 2>/dev/null || true)"
set -- $version_output
installed_version="${2:-unknown}"

config_home="${XDG_CONFIG_HOME:-${HOME:-}/.config}"
if [ -n "$config_home" ]; then
  manifest_dir="${config_home}/kubio"
  if mkdir -p "$manifest_dir" 2>/dev/null; then
    cat >"${manifest_dir}/install.json" <<EOF
{
  "schema_version": 1,
  "repo": "$(json_escape "$repo")",
  "installed_path": "$(json_escape "$install_path")",
  "target": "$target",
  "flavor": "$flavor",
  "installed_version": "$(json_escape "$installed_version")"
}
EOF
  else
    printf 'kubio install: warning: could not write install manifest under %s\n' "$manifest_dir" >&2
  fi
fi

printf 'Installed %s at %s\n' "${version_output:-kubio}" "$install_path"

case ":${PATH:-}:" in
  *":${install_dir}:"*) ;;
  *)
    printf 'Add %s to PATH to run kubio from any directory.\n' "$install_dir"
    ;;
esac
