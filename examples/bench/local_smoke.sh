#!/usr/bin/env bash
set -euo pipefail

ORIGIN_PORT="${ORIGIN_PORT:-13001}"
PROXY_PORT="${PROXY_PORT:-18081}"
DASHBOARD_PORT="${DASHBOARD_PORT:-19901}"
REQUESTS="${REQUESTS:-100}"
MODE="${MODE:-watch}"
STORAGE_KIND="${STORAGE_KIND:-memory}"

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

printf '{"message":"ok"}\n' > "${origin_dir}/index.html"
mkdir -p "${origin_dir}/api"
printf '{"products":[{"id":1,"name":"bench"}]}\n' > "${origin_dir}/api/products"
python_cmd="${PYTHON:-python3}"
"${python_cmd}" -m http.server "${ORIGIN_PORT}" --directory "${origin_dir}" >/tmp/kubio-origin.log 2>&1 &
origin_pid="$!"

origin_ready=""
for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:${ORIGIN_PORT}/api/products" >/dev/null 2>&1; then
    origin_ready="1"
    break
  fi
  sleep 0.05
done
if [[ -z "${origin_ready}" ]]; then
  echo "origin did not become ready" >&2
  tail -n 100 /tmp/kubio-origin.log >&2 || true
  exit 1
fi

if [[ "${STORAGE_KIND}" == "disk" ]]; then
  cat > "${config_file}" <<EOF
server:
  listen: "127.0.0.1:${PROXY_PORT}"
origin: "http://127.0.0.1:${ORIGIN_PORT}"
mode: "${MODE}"
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
  - match:
      method: GET
      path: "/api/products"
    query:
      ignore: ["utm_*", "gclid", "fbclid"]
EOF
  cargo run -p kubio-cli -- serve --config "${config_file}" >/tmp/kubio-bench.log 2>&1 &
else
  cargo run -p kubio-cli -- serve \
    --to "http://127.0.0.1:${ORIGIN_PORT}" \
    --listen "127.0.0.1:${PROXY_PORT}" \
    --dashboard "127.0.0.1:${DASHBOARD_PORT}" \
    --mode "${MODE}" >/tmp/kubio-bench.log 2>&1 &
fi
kubio_pid="$!"

ready=""
for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/overview" >/dev/null 2>&1; then
    ready="1"
    break
  fi
  sleep 0.05
done
if [[ -z "${ready}" ]]; then
  echo "kubio did not become ready" >&2
  tail -n 100 /tmp/kubio-bench.log >&2 || true
  exit 1
fi

start_ns="$(date +%s%N)"
for _ in $(seq 1 "${REQUESTS}"); do
  curl -fsS "http://127.0.0.1:${PROXY_PORT}/api/products?utm_source=bench" >/dev/null
done
end_ns="$(date +%s%N)"

elapsed_ms="$(( (end_ns - start_ns) / 1000000 ))"
printf 'requests=%s elapsed_ms=%s avg_ms=%s\n' "${REQUESTS}" "${elapsed_ms}" "$(( elapsed_ms / REQUESTS ))"
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/metrics" | grep -E 'kubio_requests_total|kubio_request_duration_seconds_count'
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/store" | grep -E "\"kind\":\"${STORAGE_KIND}\""
