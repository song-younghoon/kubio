#!/usr/bin/env bash
set -euo pipefail

KUBIO_BIN="${KUBIO_BIN:-target/release/kubio}"
ORIGIN_PORT="${ORIGIN_PORT:-13002}"
PROXY_PORT="${PROXY_PORT:-18082}"
DASHBOARD_PORT="${DASHBOARD_PORT:-19902}"

origin_dir="$(mktemp -d)"
cache_dir="$(mktemp -d)"
config_file="$(mktemp)"
origin_pid=""
kubio_pid=""

cleanup() {
  if [[ -n "${kubio_pid}" ]]; then
    kill "${kubio_pid}" 2>/dev/null || true
  fi
  if [[ -n "${origin_pid}" ]]; then
    kill "${origin_pid}" 2>/dev/null || true
  fi
  rm -rf "${origin_dir}" "${cache_dir}" "${config_file}"
}
trap cleanup EXIT

mkdir -p "${origin_dir}/api"
printf '{"products":[{"id":1,"name":"release-smoke"}]}\n' > "${origin_dir}/api/products"

python_cmd="${PYTHON:-python3}"
"${python_cmd}" -m http.server "${ORIGIN_PORT}" --directory "${origin_dir}" >/tmp/kubio-release-origin.log 2>&1 &
origin_pid="$!"

origin_ready=""
for _ in $(seq 1 120); do
  if curl -fsS "http://127.0.0.1:${ORIGIN_PORT}/api/products" >/dev/null 2>&1; then
    origin_ready="1"
    break
  fi
  sleep 0.05
done
if [[ -z "${origin_ready}" ]]; then
  echo "origin did not become ready" >&2
  tail -n 100 /tmp/kubio-release-origin.log >&2 || true
  exit 1
fi

cat > "${config_file}" <<EOF
server:
  listen: "127.0.0.1:${PROXY_PORT}"
origin: "http://127.0.0.1:${ORIGIN_PORT}"
mode: "auto"
dashboard:
  listen: "127.0.0.1:${DASHBOARD_PORT}"
policy:
  min_route_samples: 2
  min_key_repeats: 2
  min_shadow_validations: 1
storage:
  kind: "disk"
  path: "${cache_dir}"
  max_size: "64MiB"
  max_object_size: "1MiB"
routes:
  - name: "release products"
    match:
      method: GET
      path: "/api/products"
    query:
      ignore: ["utm_*", "gclid", "fbclid"]
    stale_if_error:
      enabled: true
      max_stale: "5m"
EOF

"${KUBIO_BIN}" serve --config "${config_file}" >/tmp/kubio-release-smoke.log 2>&1 &
kubio_pid="$!"

ready=""
for _ in $(seq 1 120); do
  if curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/overview" >/dev/null 2>&1; then
    ready="1"
    break
  fi
  sleep 0.05
done
if [[ -z "${ready}" ]]; then
  echo "kubio did not become ready" >&2
  tail -n 100 /tmp/kubio-release-smoke.log >&2 || true
  exit 1
fi

curl -fsS "http://127.0.0.1:${PROXY_PORT}/api/products?utm_source=one" >/dev/null
curl -fsS "http://127.0.0.1:${PROXY_PORT}/api/products?utm_source=two" >/dev/null
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/overview" | grep -E '"store_kind":"disk"'
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/store" | grep -E '"kind":"disk"'
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/metrics" | grep -E 'kubio_revalidation_attempts_total|kubio_cache_entries'

echo "release smoke ok"
