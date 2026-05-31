# Runtime Config Reload

Status: implemented
Target release: `v0.5.3`

## Goals

Runtime config reload lets kubio apply safe behavioral changes while the
process keeps running. The contract must be explicit enough that an operator
knows when a change can be reloaded and when a restart is required.

The reload path answers five questions:

1. Where does the new config come from?
2. Is the new file syntactically and semantically valid?
3. Does the diff contain only reloadable fields?
4. What state must be retained, demoted, or purged?
5. Which config generation is active for new requests?

## Config Source

When `kubio serve --config PATH` is used, the runtime stores:

```rust
pub struct StartupConfigSource {
    pub path: PathBuf,
    pub startup_overrides: StartupOverrides,
}
```

Startup CLI overrides such as `--to`, `--listen`, `--dashboard`, `--mode`,
`--freshness`, `--debug-headers`, and `--panic-file` are part of the effective
startup config. v0.5.3 preserves the current precedence rule on reload:

```text
defaults -> config file -> startup CLI overrides
```

This keeps reload behavior predictable. A file edit cannot silently override a
startup CLI flag that was already chosen by the operator.

If the process started without `--config`, explicit reload returns
`no_config_source`. `POST /api/config/check` and `kubio config diff --config`
may supply candidate text for dry-run validation and diffing without adopting a
new runtime config source. Applying a new config source to a running process is
out of scope.

## Reload Entry Points

### CLI

Add:

```bash
kubio config reload --dashboard http://127.0.0.1:9900
kubio config reload --dry-run --dashboard http://127.0.0.1:9900
kubio config check --config ./kubio.yml
kubio config diff --config ./kubio.yml --dashboard http://127.0.0.1:9900
kubio config status --dashboard http://127.0.0.1:9900
```

`reload` calls the dashboard admin API and applies the stored startup config
source unless `--dry-run` is set. `check` validates a local file without
contacting a running process. `diff` sends candidate file text to
`POST /api/config/check`, which rebuilds the effective candidate with startup
overrides, validates it, and diffs it against the active runtime config.
`status` reads `GET /api/config/reload-status`.

### Admin API

Add protected endpoints:

```text
POST /api/config/reload
POST /api/config/check
GET  /api/config/reload-status
GET  /api/config/active
```

`POST /api/config/reload` uses the stored startup config source by default.
The request body may optionally request dry-run mode:

```json
{"dry_run": true}
```

When `admin_token` is configured, reload endpoints require the same admin auth
as purge. `POST /api/config/check` uses the same protection. Status and active
config are read APIs and follow the dashboard read model.

### SIGHUP

On Unix, SIGHUP triggers the same reload flow as the admin API. It does not
write to stdout. It logs via tracing and records observer events.

## Reloadable Fields

v0.5.3 allows these fields to reload:

```text
mode
freshness
policy.respect_origin_headers
policy.protect_authorization
policy.protect_cookies
policy.protect_set_cookie
policy.max_object_size
policy.max_fingerprint_body_size
policy.max_request_body_size
policy.min_route_samples
policy.min_key_repeats
policy.min_shadow_validations
policy.max_shadow_mismatch_rate
policy.revalidation
policy.stale_if_error
policy.query_intelligence
policy.response_header_equivalence
policy.adaptive_reuse
routes
debug_headers
panic_file
observability.tracing
```

Reloading `observability.tracing` means toggling runtime recording where the
existing tracing infrastructure supports it. It does not imply replacing the
global subscriber.

## Restart-Required Fields

v0.5.3 rejects reloads that change these fields:

```text
server.listen
server.origin_timeout_ms
server.tls
server.protocols
server.http2
server.http3
origin
origin_protocol
dashboard.enabled
dashboard.listen
dashboard.allow_public
dashboard.admin_api
storage.kind
storage.path
storage.max_size
storage.max_object_size
storage.sync
performance
observability.metrics
observability.metrics_path
observability.max_routes
observability.max_keys
observability.max_events
admin_token
```

Rationale:

- listener and protocol fields require rebinding sockets or rebuilding protocol
  servers;
- origin fields require rebuilding clients and connection pools;
- storage fields affect existing store identity and capacity behavior;
- performance fields often size runtime structures created at startup;
- metrics path registration is part of the dashboard router;
- changing `admin_token` at runtime needs a separate credential-rotation
  design.

## Diff Model

Add a structural diff that classifies changed fields:

```rust
pub enum ConfigChangeClass {
    Reloadable,
    RestartRequired,
}

pub struct ConfigDiffEntry {
    pub path: String,
    pub class: ConfigChangeClass,
    pub summary: String,
    pub secret: bool,
}
```

The diff never includes secret values. Secret changes, currently `admin_token`,
are marked with `secret: true` and summarized without the value. Route hints
are summarized by count and exposed through bounded route diff entries rather
than raw traffic values.

Example CLI output:

```text
reloadable:
  mode: shadow -> auto
  routes: 1 added, 1 changed
  policy.response_header_equivalence: verified_ignore allowlist changed

restart required:
  server.listen: listener address changed
```

If any restart-required field changed, no reloadable fields are applied.

## Config Generation

Every committed config receives a monotonic process-local generation:

```rust
pub struct ActiveConfig {
    pub generation: u64,
    pub loaded_at_unix_ms: u64,
    pub config: Arc<EffectiveConfig>,
}
```

The shipped type is `ActiveRuntime`, which also carries the rebuilt
`PolicyEngine` and `RouteHintLookup` so the active config, policy, and route
lookup are published as one snapshot:

```rust
pub struct ActiveRuntime {
    pub generation: u64,
    pub loaded_at_unix_ms: u64,
    pub config: Arc<EffectiveConfig>,
    pub policy: Arc<PolicyEngine>,
    pub(crate) route_hints: Arc<RouteHintLookup>,
}
```

Generation `1` is startup. A successful reload increments the generation.
Failed reload attempts get attempt IDs but do not increment active generation.

Requests capture the active generation at request start and use that generation
consistently for policy, route hints, debug headers, and response handling.

## Reload Results

Reload result classes:

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

Failure response example:

```json
{
  "status": "restart_required",
  "active_generation": 3,
  "attempt_id": 4,
  "restart_required": ["server.listen"],
  "message": "config contains restart-required changes"
}
```

Success response example:

```json
{
  "status": "applied",
  "previous_generation": 3,
  "active_generation": 4,
  "reloadable_changes": 5,
  "routes_added": 1,
  "routes_changed": 2,
  "routes_removed": 0
}
```
