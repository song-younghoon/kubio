# Observability and Dashboard

Status: planned
Target release: `v0.5.3`

## Goals

v0.5.3 observability should make reload behavior auditable. Operators should be
able to answer:

- Which config generation is active?
- When did the last reload happen?
- Did the reload apply, fail validation, or require restart?
- Which field classes changed?
- Which routes were demoted or purged because of the reload?
- Are new requests using the expected generation?

## Snapshot Fields

Extend overview or add a config status snapshot:

```rust
pub struct ConfigReloadSnapshot {
    pub active_generation: u64,
    pub startup_generation: u64,
    pub config_source: Option<String>,
    pub last_attempt_id: Option<u64>,
    pub last_attempt_at_unix_ms: Option<u64>,
    pub last_status: Option<ReloadStatus>,
    pub last_reloadable_change_count: u64,
    pub last_restart_required_count: u64,
    pub last_routes_added: u64,
    pub last_routes_changed: u64,
    pub last_routes_removed: u64,
    pub last_routes_demoted: u64,
    pub last_cache_entries_purged: u64,
}
```

Allowed status values:

```text
applied
dry_run_ok
parse_failed
validation_failed
restart_required
state_reconciliation_failed
no_config_source
unauthorized
internal_error
```

Add per-route reload metadata where useful:

```rust
pub struct RouteReloadSnapshot {
    pub last_config_generation: u64,
    pub last_reload_action: Option<RouteReloadAction>,
    pub last_reload_reason: Option<String>,
}
```

Allowed route actions:

```text
unchanged
added
removed
demoted
purged
retained
requires_revalidation
```

## Dashboard

The config page should show:

```text
Active generation: 4
Config source: /etc/kubio/kubio.yml
Last reload: applied, 2026-05-31 10:42:16
Changes: 5 reloadable, 0 restart-required
Routes: 1 added, 2 changed, 0 removed, 1 demoted
Purged entries: 3
```

For a failed reload:

```text
Last reload: restart_required
Reason: server.listen changed
Active generation: 4
Attempt: 5
```

The route detail page should show when a route was demoted by reload:

```text
Config reload:
  generation: 5
  action: demoted
  reason: response header force_include changed
```

Dashboard actions may include a reload button only when `admin_api` is enabled.
If `admin_token` is configured, the UI should follow the existing admin auth
pattern. If no authenticated UI pattern exists, the button can be deferred and
the API/CLI can ship first.

## CLI

Add concise commands:

```bash
kubio config reload
kubio config check --config ./kubio.yml
kubio config diff --config ./kubio.yml
kubio config status
```

Example `status`:

```text
active_generation=4
config_source=/etc/kubio/kubio.yml
last_reload=applied
last_attempt=2026-05-31T10:42:16Z
reloadable_changes=5
restart_required=0
routes_demoted=1
cache_entries_purged=3
```

Example failed `reload`:

```text
reload rejected: restart required
active_generation=4
restart_required:
  server.listen
```

## Debug Headers

When debug headers are enabled, add:

```text
x-kubio-config-generation: 4
```

Do not add reload failure details to proxied responses. Reload failures are
control-plane state and should be visible through dashboard/API/CLI/events.

## Metrics

Add bounded metrics:

```text
kubio_config_generation
kubio_config_reload_attempts_total{status}
kubio_config_reload_changes_total{class}
kubio_config_reload_routes_total{action}
kubio_config_reload_cache_entries_purged_total
kubio_config_reload_duration_seconds_bucket
```

Labels must stay bounded. Do not label metrics by file path, route template,
header name, query parameter name, or secret field.

## Events

Add bounded events:

```text
config_reload_started
config_reload_applied
config_reload_rejected
config_reload_state_reconciled
config_reload_route_demoted
config_reload_cache_purged
```

Events may include:

- active generation;
- attempt ID;
- status;
- bounded reason;
- route ID hash;
- counts.

Events must not include:

- admin token;
- authorization header;
- cookie header;
- raw query values;
- raw response header values;
- response bodies.

## API

Add or extend:

```text
GET  /api/config
GET  /api/config/reload-status
POST /api/config/check
POST /api/config/reload
```

`GET /api/config` should include active generation alongside the existing
redacted config. If changing its response shape is too disruptive, add:

```text
GET /api/config/active
```

with:

```json
{
  "generation": 4,
  "config": {
    "admin_token": "REDACTED"
  }
}
```

## Privacy Review

Before release, use a config and traffic set containing:

```text
admin_token: raw-admin-token
Authorization: Bearer raw-secret-token
Cookie: session=raw-cookie-secret
?token=raw-query-secret
X-Vendor-Execution-Id: raw-vendor-id
```

Assert raw values do not appear in:

- reload API responses;
- config diff output;
- dashboard HTML;
- metrics;
- events;
- debug headers;
- CLI output.
