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
- Dashboard listen: `127.0.0.1:9900`
- Mode: `watch`
- Freshness: `balanced`
- Metrics path: `/metrics`
- Storage: in-memory
- Max cache size: `256MiB`
- Max object size: `1MiB`

Public dashboard binding requires explicit configuration. If admin APIs are enabled on a public dashboard address, configure an admin token and pass it to admin commands with `--admin-token` or `KUBIO_ADMIN_TOKEN`.

Observability settings:

```yaml
observability:
  metrics: true
  metrics_path: "/metrics"
```

`metrics_path` must be an absolute dashboard path such as `/metrics` or `/internal/metrics`.
