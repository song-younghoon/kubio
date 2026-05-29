#!/usr/bin/env bash
set -euo pipefail

ORIGIN_PORT="${ORIGIN_PORT:-13001}"
PROXY_PORT="${PROXY_PORT:-18081}"
DASHBOARD_PORT="${DASHBOARD_PORT:-19901}"
REQUESTS="${REQUESTS:-100}"

origin_dir="$(mktemp -d)"
origin_pid=""
kubio_pid=""

cleanup() {
  if [[ -n "${kubio_pid}" ]]; then
    kill "${kubio_pid}" 2>/dev/null || true
  fi
  if [[ -n "${origin_pid}" ]]; then
    kill "${origin_pid}" 2>/dev/null || true
  fi
  rm -rf "${origin_dir}"
}
trap cleanup EXIT

printf '{"message":"ok"}\n' > "${origin_dir}/index.html"
python_cmd="${PYTHON:-python3}"
"${python_cmd}" -m http.server "${ORIGIN_PORT}" --directory "${origin_dir}" >/tmp/kubio-origin.log 2>&1 &
origin_pid="$!"

cargo run -p kubio-cli -- serve \
  --to "http://127.0.0.1:${ORIGIN_PORT}" \
  --listen "127.0.0.1:${PROXY_PORT}" \
  --dashboard "127.0.0.1:${DASHBOARD_PORT}" \
  --mode watch >/tmp/kubio-bench.log 2>&1 &
kubio_pid="$!"

for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/overview" >/dev/null 2>&1; then
    break
  fi
  sleep 0.05
done

start_ns="$(date +%s%N)"
for _ in $(seq 1 "${REQUESTS}"); do
  curl -fsS "http://127.0.0.1:${PROXY_PORT}/" >/dev/null
done
end_ns="$(date +%s%N)"

elapsed_ms="$(( (end_ns - start_ns) / 1000000 ))"
printf 'requests=%s elapsed_ms=%s avg_ms=%s\n' "${REQUESTS}" "${elapsed_ms}" "$(( elapsed_ms / REQUESTS ))"
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/metrics" | grep -E 'kubio_requests_total|kubio_request_duration_seconds_count'
