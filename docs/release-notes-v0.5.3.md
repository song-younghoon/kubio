# v0.5.3 Release Notes

v0.5.3 adds runtime config reload for safe behavioral changes.

## Added

- Runtime config generations for proxy requests.
- Paired config, policy, and route-hint swaps for new requests.
- `kubio config check`, `kubio config reload`, `kubio config diff`, and
  `kubio config status`.
- Protected admin endpoints:
  - `GET /api/config/active`
  - `GET /api/config/reload-status`
  - `POST /api/config/reload`
  - `POST /api/config/check`
- Unix SIGHUP reload when kubio started with `--config`.
- Reload dashboard fields, observer events, debug header
  `x-kubio-config-generation`, and bounded Prometheus metrics.
- Conservative state reconciliation that purges/demotes affected routes before
  publishing a new generation.
- Reload smoke benchmark scenario: `kubio-bench --scenario reload-smoke`.

## Reloadable

- `mode`
- `freshness`
- `policy.*`
- `routes`
- `debug_headers`
- `panic_file`
- `observability.tracing`

## Restart Required

Listener, TLS, protocol, origin, dashboard binding, storage, performance,
metrics path, and `admin_token` changes still require restart. Mixed diffs are
rejected without applying the reloadable subset.

## Safety

Failed reloads keep the old active generation. Admin token values stay redacted
in config output and diff results.
