# Configuration

kubio works with one required value: the origin URL.

```bash
kubio serve --to http://localhost:3000
```

Optional YAML config:

```bash
kubio serve --config examples/kubio.yml
```

CLI flags override config file values.

Important defaults:

- Proxy listen: `0.0.0.0:8080`
- Origin timeout: `30000` ms
- Dashboard listen: `127.0.0.1:9900`
- Mode: `watch`
- Freshness: `balanced`
- Metrics path: `/metrics`
- Storage: in-memory
- Max cache size: `256MiB`
- Max object size: `1MiB`
- Revalidation: enabled
- Stale-if-error: origin-controlled
- Downstream HTTP/2: disabled unless TLS or explicit h2c config enables it
- Downstream HTTP/3: disabled and guarded in the default build

Server settings:

```yaml
server:
  listen: "0.0.0.0:8080"
  origin_timeout_ms: 30000
```

v0.3.0 protocol settings:

```yaml
server:
  listen: "0.0.0.0:8443"
  tls:
    cert: "certs/kubio.pem"
    key: "certs/kubio-key.pem"
  protocols:
    http1: true
    http2: true
    h2c: false
  http2:
    max_concurrent_streams: 256
    initial_stream_window_size: "1MiB"
    initial_connection_window_size: "4MiB"
    keepalive_timeout: "10s"
    max_header_list_size: "64KiB"

origin_protocol:
  preferred: "auto" # auto | http1 | http2
  fallback: true
  http2_prior_knowledge: false
```

When `origin_protocol.fallback` is false, kubio fails closed with a gateway error if the origin response does not use the required protocol. Negotiated fallback is recorded in metrics and events when fallback is enabled.

For local cleartext HTTP/2 prior knowledge:

```yaml
server:
  protocols:
    http1: true
    http2: true
    h2c: true
```

HTTP/3 config is parsed but guarded in the default v0.3.0 build. Setting `server.http3.enabled: true` or `origin_protocol.http3_experimental: true` fails startup with a clear message.

Performance settings:

```yaml
performance:
  max_in_flight_requests: 4096
  max_buffered_response_size: "2MiB"
  stream_unstoreable_bodies: true
  observer_shards: 64
  async_disk_writes: true
  origin_pool_max_idle_per_host: 32
  origin_pool_idle_timeout: "90s"
```

Public dashboard binding requires explicit configuration. If admin APIs are enabled on a public dashboard address, configure an admin token and pass it to admin commands with `--admin-token` or `KUBIO_ADMIN_TOKEN`.

Observability settings:

```yaml
observability:
  metrics: true
  metrics_path: "/metrics"
```

`metrics_path` must be an absolute dashboard path such as `/metrics` or `/internal/metrics`.

v0.3.0 observability includes downstream/upstream protocol counts, protocol fallback counts, live in-flight gauges, backpressure rejections, store operation counters/latency totals, store saturation events, and observer event-drop counts.

v0.2.0 policy settings:

```yaml
policy:
  revalidation:
    enabled: true
    prefer_etag: true
    max_validator_length: 1024
  stale_if_error:
    mode: "origin"
    max_stale: "5m"
  query_intelligence:
    enabled: true
    auto_ignore: false
```

Disk store:

```yaml
storage:
  kind: "disk"
  path: ".kubio/cache"
  max_size: "1GiB"
  max_object_size: "2MiB"
```

Route hints:

```yaml
routes:
  - match:
      method: GET
      path: "/api/products"
    query:
      ignore: ["utm_*", "gclid"]
    stale_if_error:
      enabled: true
      max_stale: "5m"
```
