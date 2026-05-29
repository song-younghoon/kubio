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

Server settings:

```yaml
server:
  listen: "0.0.0.0:8080"
  origin_timeout_ms: 30000
```

Public dashboard binding requires explicit configuration. If admin APIs are enabled on a public dashboard address, configure an admin token and pass it to admin commands with `--admin-token` or `KUBIO_ADMIN_TOKEN`.

Observability settings:

```yaml
observability:
  metrics: true
  metrics_path: "/metrics"
```

`metrics_path` must be an absolute dashboard path such as `/metrics` or `/internal/metrics`.

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
