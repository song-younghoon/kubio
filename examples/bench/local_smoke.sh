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
overview_file=""
metrics_file=""
origin_pid=""
kubio_pid=""

cleanup() {
  if [[ -n "${kubio_pid}" ]]; then
    kill "${kubio_pid}" 2>/dev/null || true
  fi
  if [[ -n "${origin_pid}" ]]; then
    kill "${origin_pid}" 2>/dev/null || true
  fi
  if [[ -n "${overview_file}" ]]; then
    rm -f "${overview_file}"
  fi
  if [[ -n "${metrics_file}" ]]; then
    rm -f "${metrics_file}"
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

storage_path_line=""
if [[ "${STORAGE_KIND}" == "disk" ]]; then
  storage_path_line="  path: \"${cache_dir}\""
fi

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
  kind: "${STORAGE_KIND}"
${storage_path_line}
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
overview_file="$(mktemp)"
metrics_file="$(mktemp)"
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/overview" > "${overview_file}"
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/metrics" > "${metrics_file}"
"${python_cmd}" - "${REQUESTS}" "${elapsed_ms}" "${overview_file}" <<'PY'
import json
import sys

requests = int(sys.argv[1])
elapsed_ms = int(sys.argv[2])
with open(sys.argv[3], encoding="utf-8") as handle:
    overview = json.load(handle)
result = {
    "requests": requests,
    "elapsed_ms": elapsed_ms,
    "avg_ms": elapsed_ms / requests if requests else 0,
    "observed_requests": overview.get("observed_requests", 0),
    "origin_requests": overview.get("origin_requests", 0),
    "reused_responses": overview.get("reused_responses", 0),
    "protected_requests": overview.get("protected_requests", 0),
    "revalidation_attempts": overview.get("revalidation_attempts", 0),
    "stale_responses_served": overview.get("stale_responses_served", 0),
    "backpressure_rejections": overview.get("backpressure_rejections", 0),
    "protocol_fallbacks": overview.get("protocol_fallbacks", 0),
    "downstream": {
        "http1": overview.get("downstream_http1_requests", 0),
        "http2": overview.get("downstream_http2_requests", 0),
        "http3": overview.get("downstream_http3_requests", 0),
    },
    "upstream": {
        "http1": overview.get("upstream_http1_requests", 0),
        "http2": overview.get("upstream_http2_requests", 0),
        "http3": overview.get("upstream_http3_requests", 0),
    },
    "p50_latency_ms": overview.get("p50_latency_ms", 0),
    "p95_latency_ms": overview.get("p95_latency_ms", 0),
}
print(json.dumps(result, sort_keys=True))
PY
grep -E 'kubio_requests_total|kubio_request_duration_seconds_count|kubio_downstream_requests_total|kubio_upstream_requests_total' "${metrics_file}"
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/store" | grep -E "\"kind\":\"${STORAGE_KIND}\""
