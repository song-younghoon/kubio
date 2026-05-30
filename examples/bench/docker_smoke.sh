#!/usr/bin/env bash
set -euo pipefail

KUBIO_IMAGE="${KUBIO_IMAGE:-kubio:ci}"
ORIGIN_PORT="${ORIGIN_PORT:-13003}"
PROXY_PORT="${PROXY_PORT:-18083}"
DASHBOARD_PORT="${DASHBOARD_PORT:-19903}"
ORIGIN_IMAGE="${ORIGIN_IMAGE:-python:3-alpine}"
DOCKER_USER="${DOCKER_USER:-$(id -u):$(id -g)}"

origin_dir="$(mktemp -d)"
cache_dir="$(mktemp -d)"
config_file="$(mktemp)"
http3_config_file="$(mktemp)"
chmod 0777 "${cache_dir}"
chmod 0644 "${config_file}" "${http3_config_file}"
network_name="kubio-smoke-$RANDOM-$$"
origin_name="kubio-smoke-origin-$RANDOM-$$"
origin_container_id=""
container_id=""

cleanup() {
  if [[ -n "${container_id}" ]]; then
    docker rm -f "${container_id}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${origin_container_id}" ]]; then
    docker rm -f "${origin_container_id}" >/dev/null 2>&1 || true
  fi
  if [[ -d "${cache_dir}" ]]; then
    docker run --rm \
      --entrypoint sh \
      -v "${cache_dir}:/cache" \
      "${ORIGIN_IMAGE}" -c 'find /cache -mindepth 1 -exec rm -rf {} +' \
      >/dev/null 2>&1 || true
  fi
  docker network rm "${network_name}" >/dev/null 2>&1 || true
  rm -rf "${origin_dir}" "${cache_dir}" "${config_file}" "${http3_config_file}"
}
trap cleanup EXIT

mkdir -p "${origin_dir}/api"
printf '{"products":[{"id":1,"name":"docker-smoke"}]}\n' > "${origin_dir}/api/products"

docker network create "${network_name}" >/dev/null
origin_container_id="$(
  docker run -d \
    --name "${origin_name}" \
    --network "${network_name}" \
    -p "127.0.0.1:${ORIGIN_PORT}:8000" \
    -v "${origin_dir}:/origin:ro" \
    "${ORIGIN_IMAGE}" python -m http.server 8000 --directory /origin
)"

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
  docker logs "${origin_container_id}" >&2 || true
  exit 1
fi

cat > "${config_file}" <<EOF
server:
  listen: "0.0.0.0:${PROXY_PORT}"
origin: "http://${origin_name}:8000"
mode: "auto"
dashboard:
  listen: "0.0.0.0:${DASHBOARD_PORT}"
  allow_public: true
  admin_api: false
policy:
  min_route_samples: 2
  min_key_repeats: 2
  min_shadow_validations: 1
storage:
  kind: "disk"
  path: "/cache"
  max_size: "64MiB"
  max_object_size: "1MiB"
routes:
  - match:
      method: GET
      path: "/api/products"
    query:
      ignore: ["utm_*", "gclid", "fbclid"]
EOF

container_id="$(
  docker run -d \
    --network "${network_name}" \
    --user "${DOCKER_USER}" \
    -p "127.0.0.1:${PROXY_PORT}:${PROXY_PORT}" \
    -p "127.0.0.1:${DASHBOARD_PORT}:${DASHBOARD_PORT}" \
    -v "${config_file}:/config.yml:ro" \
    -v "${cache_dir}:/cache" \
    "${KUBIO_IMAGE}" serve --config /config.yml
)"

ready=""
for _ in $(seq 1 120); do
  if curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/overview" >/dev/null 2>&1; then
    ready="1"
    break
  fi
  sleep 0.05
done
if [[ -z "${ready}" ]]; then
  echo "kubio container did not become ready" >&2
  docker logs "${container_id}" >&2 || true
  exit 1
fi

curl -fsS "http://127.0.0.1:${PROXY_PORT}/api/products?utm_source=one" >/dev/null
curl -fsS "http://127.0.0.1:${PROXY_PORT}/api/products?utm_source=two" >/dev/null
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/api/store" | grep -E '"kind":"disk"'
curl -fsS "http://127.0.0.1:${DASHBOARD_PORT}/metrics" | grep -E 'kubio_cache_entries|kubio_query_hints_applied_total'

cat > "${http3_config_file}" <<EOF
server:
  listen: "0.0.0.0:${PROXY_PORT}"
  tls:
    cert: "/missing/localhost.pem"
    key: "/missing/localhost-key.pem"
  http3:
    enabled: true
    advertise: true
    authorities:
      - "localhost:${PROXY_PORT}"
origin: "http://${origin_name}:8000"
EOF

if docker run --rm \
  --network "${network_name}" \
  -v "${http3_config_file}:/http3.yml:ro" \
  "${KUBIO_IMAGE}" serve --config /http3.yml >/tmp/kubio-http3-docker-smoke.out 2>&1; then
  echo "HTTP/3 guarded config unexpectedly started in the default docker image" >&2
  cat /tmp/kubio-http3-docker-smoke.out >&2
  exit 1
fi
grep -E 'HTTP/3 runtime requires|TLS cert|localhost.pem' /tmp/kubio-http3-docker-smoke.out >/dev/null

echo "docker smoke ok"
