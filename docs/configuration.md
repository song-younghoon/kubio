# Configuration

kubio works with one required value: the origin URL.

```bash
kubio serve --to http://localhost:3000
```

Optional YAML config:

```bash
kubio serve --config examples/kubio.yml
```

CLI flags override config file values. Runtime reload keeps that precedence:
defaults, then the config file, then the original startup CLI overrides.

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
- Adaptive reuse: enabled
- Downstream HTTP/2: disabled unless TLS or explicit h2c config enables it
- Downstream HTTP/3: disabled by default; available only in binaries built with
  `--features experimental-http3`

Server settings:

```yaml
server:
  listen: "0.0.0.0:8080"
  origin_timeout_ms: 30000
```

Protocol settings:

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
  http3:
    enabled: false
    listen: "0.0.0.0:8443"
    advertise: false
    authorities:
      - "api.example.com:443"
    alt_svc_ma: "1h"
    max_concurrent_streams: 128
    max_field_section_size: "64KiB"
    qpack_max_table_capacity: "0"
    max_udp_payload_size: "1350"
    idle_timeout: "30s"

origin_protocol:
  preferred: "auto" # auto | http1 | http2 | http3
  fallback: true
  http2_prior_knowledge: false
  http3_experimental: false
  http3_max_idle_connections: 32
  http3_idle_timeout: "90s"
  http3_ca_certs: []
```

HTTP/2 server settings are applied to downstream connections through Hyper's connection builder. When `origin_protocol.fallback` is false, kubio fails closed with a gateway error if the origin response does not use the required protocol. Negotiated fallback is recorded in metrics and events when fallback is enabled, including retry fallback from HTTP/2 prior-knowledge connection failures to HTTP/1.1 for replayable safe requests.

For local cleartext HTTP/2 prior knowledge:

```yaml
server:
  protocols:
    http1: true
    http2: true
    h2c: true
```

HTTP/3 config is parsed and validated. The downstream QUIC listener is available
only in binaries built with `--features experimental-http3`, requires
`server.tls`, and listens on `server.http3.listen` or falls back to
`server.listen` as a UDP socket. `server.http3.qpack_max_table_capacity` must
remain `0` in the current runtime slice. `server.http3.advertise` is opt-in and
emits `Alt-Svc` only when the request authority exactly matches one of
`server.http3.authorities`; kubio strips origin `Alt-Svc` instead of forwarding
it for unconfigured authorities.

Upstream HTTP/3 is experimental and only attempted for HTTPS origins when the
binary is built with `--features experimental-http3`,
`origin_protocol.preferred: "http3"`, and
`origin_protocol.http3_experimental: true`. Replayable GET/HEAD failures can
fall back when `origin_protocol.fallback: true`; non-replayable fallback is
blocked before bytes are sent. `origin_protocol.http3_ca_certs` accepts
additional PEM trust anchors for private origins.

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

v0.3.1 observability includes downstream/upstream protocol counts, protocol fallback counts, HTTP/3 connection/stream/write-error counters, QUIC handshake failure counters, Alt-Svc advertised/skipped counters, upstream HTTP/3 attempt/fallback counters, live in-flight gauges, backpressure rejections, store operation counters/latency totals, store saturation events, and observer event-drop counts.

Policy settings:

```yaml
policy:
  adaptive_reuse:
    enabled: true
    key_validation:
      min_observations: 2
      min_shadow_matches: 1
      max_shadow_mismatches: 0
    public_object:
      enabled: true
      min_route_samples: 20
      min_distinct_keys: 3
      min_store_safe_rate: 0.98
      min_shadow_matches: 3
      max_shadow_mismatches: 0
    origin_public_fast_path:
      enabled: true
    precision:
      enabled: true
      confidence:
        fresh_window_secs: 1800
        min_window_samples: 20
        strong_window_samples: 100
        max_negative_events: 0
        cooldown_secs: 600
        max_cooldown_secs: 3600
        cooldown_backoff: 2.0
      query_equivalence:
        enabled: true
        auto_compact: false
        min_distinct_values: 3
        min_matching_fingerprints: 3
        min_base_keys: 1
        max_mismatches: 0
      canary:
        enabled: true
        probation_rate: 0.10
        validated_rate: 0.02
        strong_rate: 0.005
        min_interval_secs: 30
      slug:
        enabled: true
        min_distinct_values: 3
        min_route_samples: 20
      variants:
        max_values_per_dimension: 8
        require_variant_evidence: true
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
  response_header_equivalence:
    enabled: true
    verified_ignore:
      enabled: true
      auto_apply_known_metadata: true
      auto_apply_unknown: false
      min_distinct_values: 3
      min_matching_fingerprints: 3
      max_mismatches: 0
      cooldown_secs: 600
    serve:
      strip_volatile_on_hit: true
      strip_verified_ignored_on_hit: true
      add_age: true
      preserve_date: true
    default_volatile:
      add: []
      block: []
```

`response_header_equivalence` ignores curated response metadata such as
`x-response-id`, `x-correlation-id`, and trace headers for fingerprinting while
keeping cache-safety, validator, and representation headers fingerprinted.
Cache-hit responses strip one-shot volatile metadata by default.
Set `auto_apply_known_metadata: false` to keep the curated volatile metadata set
inside fingerprints unless a route or explicit `default_volatile.add` entry opts
it in.

## Runtime Reload

When the process starts with `--config`, kubio stores the config path and the
startup CLI overrides. These commands use the running dashboard API:

```bash
kubio config reload
kubio config reload --dry-run
kubio config diff --config examples/kubio-v0.5.3-reload.yml
kubio config status
```

On Unix, SIGHUP triggers the same reload flow and records the result through
tracing and observer events. Reload attempts are serialized. Failed reloads keep
the previous active generation serving traffic.

Reloadable fields:

- `mode`
- `freshness`
- `policy.*`
- `routes`
- `debug_headers`
- `panic_file`
- `observability.tracing`

Restart-required fields:

- `server.*`
- `origin`
- `origin_protocol.*`
- `dashboard.*`
- `storage.*`
- `performance.*`
- `observability.metrics`
- `observability.metrics_path`
- `admin_token`

Mixed diffs are rejected as a whole. `admin_token` changes are reported without
exposing the token value. `GET /api/config/active` returns the redacted active
config with its generation, and `GET /api/config/reload-status` returns the last
reload result.

Example workflow:

```bash
kubio serve --config examples/kubio-v0.5.3-reload.yml
kubio config diff --config ./kubio.yml
kubio config reload
kubio config status
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
  - match:
      method: GET
      path: "/notice/{id}"
    query:
      verified_ignore:
        enabled: true
        allow: ["utm_*", "gclid", "fbclid"]
    response_headers:
      verified_ignore:
        enabled: true
        allow: ["x-vendor-execution-id"]
      force_include: ["etag"]
      preserve_on_hit: []
    safety:
      public_object: true
```

`safety.public_object: true` lowers the evidence path for known public object
routes but does not bypass hard denies such as Authorization, Cookie,
Set-Cookie, private/no-store, unsupported Vary, sensitive paths, or shadow
mismatches.

`query.verified_ignore` is stricter than `query.ignore`. kubio first has to
observe matching fingerprints across different bounded query value hashes. Only
then will enabled `verified_ignore.allow` patterns compact cache keys. Sensitive
query names such as `token`, `session`, `jwt`, `api_key`, `secret`, `signature`,
and `code` are never automatic verified-ignore candidates.

`response_headers.verified_ignore` can opt a route into ignoring known
non-semantic response metadata such as vendor execution IDs. The name still has
to be candidate-eligible; hard safety, validator, freshness, representation, and
sensitive/business-state headers cannot be made ignorable. `force_include` keeps
a header fingerprinted even if it would otherwise be treated as volatile.
