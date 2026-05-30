#!/usr/bin/env bash
set -euo pipefail

ORIGIN_PORT="${ORIGIN_PORT:-13004}"
PROXY_PORT="${PROXY_PORT:-18084}"
DASHBOARD_PORT="${DASHBOARD_PORT:-19904}"
DISK_PROXY_PORT="${DISK_PROXY_PORT:-18085}"
DISK_DASHBOARD_PORT="${DISK_DASHBOARD_PORT:-19905}"
KUBIO_READY_ATTEMPTS="${KUBIO_READY_ATTEMPTS:-1200}"

work_dir="$(mktemp -d)"
origin_app="${work_dir}/origin.py"
memory_config="${work_dir}/memory.yml"
disk_config="${work_dir}/disk.yml"
disk_cache="${work_dir}/cache"
origin_pid=""
memory_pid=""
disk_pid=""

cleanup() {
  if [[ -n "${memory_pid}" ]]; then
    kill "${memory_pid}" 2>/dev/null || true
  fi
  if [[ -n "${disk_pid}" ]]; then
    kill "${disk_pid}" 2>/dev/null || true
  fi
  if [[ -n "${origin_pid}" ]]; then
    kill "${origin_pid}" 2>/dev/null || true
  fi
  rm -rf "${work_dir}"
}
trap cleanup EXIT

cat > "${origin_app}" <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import sys

LARGE_BODY = b"x" * 8192

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, _format, *args):
        return

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path in ("/safe", "/safe-disk"):
            self.respond(200, b"safe", {"cache-control": "public, max-age=60"})
        elif path == "/protected":
            self.respond(200, b"protected", {"cache-control": "public, max-age=60"})
        elif path == "/etag":
            if self.headers.get("if-none-match"):
                self.respond(304, b"", {"etag": '"etag-v1"', "cache-control": "max-age=0"})
            else:
                self.respond(200, b"etag-body", {"etag": '"etag-v1"', "cache-control": "max-age=0"})
        elif path == "/stale-error":
            if self.headers.get("if-none-match"):
                self.respond(500, b"origin-error", {"etag": '"stale-v1"', "cache-control": "max-age=0"})
            else:
                self.respond(200, b"stale-body", {"etag": '"stale-v1"', "cache-control": "max-age=0, stale-if-error=60"})
        elif path == "/large-private":
            self.respond(200, LARGE_BODY, {"cache-control": "private"})
        else:
            self.respond(404, b"not found", {})

    def respond(self, status, body, headers):
        self.send_response(status)
        for name, value in headers.items():
            self.send_header(name, value)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

port = int(sys.argv[1])
ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
PY

python_cmd="${PYTHON:-python3}"
"${python_cmd}" "${origin_app}" "${ORIGIN_PORT}" >/tmp/kubio-baseline-origin.log 2>&1 &
origin_pid="$!"

for _ in $(seq 1 120); do
  if curl -fsS "http://127.0.0.1:${ORIGIN_PORT}/safe" >/dev/null 2>&1; then
    break
  fi
  sleep 0.05
done

write_config() {
  local file="$1"
  local proxy_port="$2"
  local dashboard_port="$3"
  local storage_kind="$4"
  local storage_path="$5"

  cat > "${file}" <<EOF
server:
  listen: "127.0.0.1:${proxy_port}"
origin: "http://127.0.0.1:${ORIGIN_PORT}"
mode: "auto"
dashboard:
  listen: "127.0.0.1:${dashboard_port}"
policy:
  min_route_samples: 2
  min_key_repeats: 2
  min_shadow_validations: 1
  stale_if_error:
    mode: "origin"
    max_stale: "5m"
storage:
  kind: "${storage_kind}"
  path: "${storage_path}"
  max_size: "64MiB"
  max_object_size: "1MiB"
performance:
  max_buffered_response_size: "1MiB"
routes:
  - match:
      method: GET
      path: "/safe"
  - match:
      method: GET
      path: "/safe-disk"
  - match:
      method: GET
      path: "/etag"
  - match:
      method: GET
      path: "/stale-error"
EOF
}

start_kubio() {
  local config="$1"
  local dashboard_port="$2"
  cargo run -p kubio-cli -- serve --config "${config}" >/tmp/kubio-baseline-smoke.log 2>&1 &
  local pid="$!"
  for _ in $(seq 1 "${KUBIO_READY_ATTEMPTS}"); do
    if curl -fsS "http://127.0.0.1:${dashboard_port}/api/overview" >/dev/null 2>&1; then
      echo "${pid}"
      return
    fi
    sleep 0.05
  done
  echo "kubio did not become ready" >&2
  tail -n 100 /tmp/kubio-baseline-smoke.log >&2 || true
  exit 1
}

write_config "${memory_config}" "${PROXY_PORT}" "${DASHBOARD_PORT}" "memory" "${work_dir}/unused"
memory_pid="$(start_kubio "${memory_config}" "${DASHBOARD_PORT}")"

curl -fsS "http://127.0.0.1:${PROXY_PORT}/safe" >/dev/null
curl -fsS -H "authorization: Bearer redacted" "http://127.0.0.1:${PROXY_PORT}/protected" >/dev/null
curl -fsS "http://127.0.0.1:${PROXY_PORT}/safe" >/dev/null
curl -fsS "http://127.0.0.1:${PROXY_PORT}/safe" >/dev/null
for _ in 1 2 3; do
  curl -fsS "http://127.0.0.1:${PROXY_PORT}/etag" >/dev/null
done
for _ in 1 2 3; do
  curl -fsS "http://127.0.0.1:${PROXY_PORT}/stale-error" >/dev/null
done
curl -fsS "http://127.0.0.1:${PROXY_PORT}/large-private" >/dev/null
for _ in $(seq 1 10); do
  curl -fsS "http://127.0.0.1:${PROXY_PORT}/safe?load=${_}" >/dev/null
done

memory_overview="${work_dir}/memory-overview.json"
memory_metrics="${work_dir}/memory-metrics.txt"
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/overview" > "${memory_overview}"
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/metrics" > "${memory_metrics}"
grep -E 'kubio_requests_total|kubio_downstream_requests_total|kubio_request_duration_seconds_count' "${memory_metrics}" >/dev/null

write_config "${disk_config}" "${DISK_PROXY_PORT}" "${DISK_DASHBOARD_PORT}" "disk" "${disk_cache}"
disk_pid="$(start_kubio "${disk_config}" "${DISK_DASHBOARD_PORT}")"
for _ in 1 2 3; do
  curl -fsS "http://127.0.0.1:${DISK_PROXY_PORT}/safe-disk" >/dev/null
done
disk_overview="${work_dir}/disk-overview.json"
curl -fsS "http://127.0.0.1:${DISK_DASHBOARD_PORT}/api/overview" > "${disk_overview}"

"${python_cmd}" - "${memory_overview}" "${disk_overview}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    memory = json.load(handle)
with open(sys.argv[2], encoding="utf-8") as handle:
    disk = json.load(handle)

checks = {
    "http1_pass_through_safe_get": memory.get("origin_requests", 0) >= 1,
    "http1_protected_request": memory.get("protected_requests", 0) >= 1,
    "http1_fresh_memory_hit": memory.get("reused_responses", 0) >= 1,
    "http1_fresh_disk_hit": disk.get("store_kind") == "disk" and disk.get("reused_responses", 0) >= 1,
    "http1_304_revalidation": memory.get("revalidation_not_modified", 0) >= 1,
    "http1_stale_if_error": memory.get("stale_responses_served", 0) >= 1,
    "large_unstoreable_response": memory.get("protected_requests", 0) >= 2,
    "metrics_render_under_load": memory.get("downstream_http1_requests", 0) >= 10,
}
result = {
    "checks": checks,
    "memory": {
        "observed_requests": memory.get("observed_requests", 0),
        "origin_requests": memory.get("origin_requests", 0),
        "reused_responses": memory.get("reused_responses", 0),
        "protected_requests": memory.get("protected_requests", 0),
        "revalidation_not_modified": memory.get("revalidation_not_modified", 0),
        "stale_responses_served": memory.get("stale_responses_served", 0),
        "downstream_http1_requests": memory.get("downstream_http1_requests", 0),
    },
    "disk": {
        "store_kind": disk.get("store_kind"),
        "observed_requests": disk.get("observed_requests", 0),
        "origin_requests": disk.get("origin_requests", 0),
        "reused_responses": disk.get("reused_responses", 0),
    },
}
print(json.dumps(result, sort_keys=True))
if not all(checks.values()):
    raise SystemExit(1)
PY
