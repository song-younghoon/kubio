# Architecture Delta

Status: design draft
Target release: `v0.3.0`

## Goals

v0.3.0 should add performance improvements and modern HTTP protocol support without replacing the v0.2.0 safety architecture.

The main architectural change is a transport boundary:

```text
protocol listener -> normalized request -> policy/cache/observer/store -> normalized response -> protocol writer
```

Policy, cache keys, route hints, stale-if-error, and observer state should remain protocol-independent.

## Workspace Changes

Existing crates remain:

```text
kubio-cli
kubio-core
kubio-proxy
kubio-policy
kubio-observe
kubio-store
kubio-dashboard
kubio-telemetry
```

Expected responsibility changes:

- `kubio-core`
  - Add protocol config, TLS config, performance config, protocol version enums, and new decision/event reasons.
- `kubio-proxy`
  - Split listener/runtime setup from request handling.
  - Add HTTP/2 configurable accept loop.
  - Add optional HTTP/3 QUIC listener.
  - Add an origin client facade for HTTP/1.1, HTTP/2, and experimental HTTP/3.
- `kubio-policy`
  - Keep request/response safety protocol-independent.
  - Add handling for protocol-specific edge cases such as trailers and malformed pseudo-header translation.
- `kubio-observe`
  - Reduce lock contention and record protocol-level stats.
  - Keep event and label cardinality bounded.
- `kubio-store`
  - Move blocking disk I/O off runtime worker threads.
  - Optionally expose metadata-first reads to avoid cloning or loading bodies before reuse is possible.
- `kubio-dashboard`
  - Surface protocol mix, connection/stream counts, benchmark snapshots, and performance warnings.
- `kubio-cli`
  - Parse protocol/TLS/performance config.
  - Extend `doctor`, `routes`, and `explain` with protocol health and benchmark hints.

A new crate is optional:

```text
kubio-transport
```

Use a new crate only if HTTP/3 code materially complicates `kubio-proxy`. Otherwise keep transport modules under `kubio-proxy`.

## Core Type Additions

### Protocol Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpProtocol {
    Http1,
    Http2,
    Http3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolSide {
    Downstream,
    Upstream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginProtocolPreference {
    Auto,
    Http1,
    Http2,
    Http3,
}
```

### Server Protocol Config

```rust
pub struct ServerProtocolConfig {
    pub http1: bool,
    pub http2: bool,
    pub h2c: bool,
    pub http3: Http3ServerConfig,
}

pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub alpn: Vec<String>,
}

pub struct Http3ServerConfig {
    pub enabled: bool,
    pub listen: Option<SocketAddr>,
    pub advertise: bool,
    pub alt_svc_ma: Duration,
    pub max_concurrent_streams: u64,
    pub max_field_section_size: usize,
    pub idle_timeout: Duration,
}
```

### Origin Protocol Config

```rust
pub struct OriginProtocolConfig {
    pub preferred: OriginProtocolPreference,
    pub fallback: bool,
    pub http2_prior_knowledge: bool,
    pub http3_experimental: bool,
}
```

### Performance Config

```rust
pub struct PerformanceConfig {
    pub max_in_flight_requests: usize,
    pub max_buffered_response_size: u64,
    pub stream_unstoreable_bodies: bool,
    pub observer_shards: usize,
    pub async_disk_writes: bool,
    pub origin_pool_max_idle_per_host: usize,
    pub origin_pool_idle_timeout: Duration,
}
```

Defaults should preserve v0.2.0 behavior while enabling safer streaming improvements:

- `stream_unstoreable_bodies: true`
- `async_disk_writes: true` for disk store
- HTTP/3 disabled
- HTTP/2 enabled only where the listener/origin stack can support it safely

## Config Model

v0.3.0 extends YAML config:

```yaml
server:
  listen: "0.0.0.0:8443"
  origin_timeout_ms: 30000
  tls:
    cert: "certs/kubio.pem"
    key: "certs/kubio-key.pem"
  protocols:
    http1: true
    http2: true
    h2c: false
  http3:
    enabled: false
    listen: "0.0.0.0:8443"
    advertise: false
    alt_svc_ma: "1h"
    max_concurrent_streams: 128

origin: "https://api.example.com"
origin_protocol:
  preferred: "auto" # auto | http1 | http2 | http3
  fallback: true
  http2_prior_knowledge: false
  http3_experimental: false

performance:
  max_in_flight_requests: 4096
  max_buffered_response_size: "2MiB"
  stream_unstoreable_bodies: true
  observer_shards: 64
  async_disk_writes: true
  origin_pool_max_idle_per_host: 32
  origin_pool_idle_timeout: "90s"
```

Validation rules:

- `server.tls` is required for browser-usable HTTP/2 through ALPN.
- `server.protocols.http2: true` without TLS is allowed only for h2c prior-knowledge mode.
- `server.protocols.h2c: true` must be explicit.
- `server.http3.enabled: true` requires TLS certificate and key.
- `server.http3.advertise: true` requires `server.http3.enabled: true`.
- HTTP/3 and TLS should use the same certificate identity unless a future advanced config explicitly separates them.
- `origin_protocol.preferred: http3` requires `origin_protocol.http3_experimental: true`.
- If `origin_protocol.fallback: false`, startup should validate that the requested protocol is configured and supported by the build.
- Performance limits must be nonzero and capped to avoid accidental unbounded memory use.

## Proxy Runtime Changes

### Current v0.2.0 Shape

```text
TcpListener
  -> axum::serve
  -> proxy_handler
  -> reqwest::Client
```

This is intentionally simple but leaves little control over HTTP/2 settings, TLS ALPN, connection metadata, and HTTP/3.

### v0.3.0 Shape

```text
TCP listener
  -> optional TLS acceptor with ALPN
  -> hyper/hyper-util HTTP/1.1 or HTTP/2 connection
  -> tower service
  -> protocol-neutral proxy handler

UDP listener
  -> quinn endpoint
  -> h3 server connection
  -> protocol-neutral proxy handler

OriginClient
  -> reqwest HTTP/1.1/HTTP/2 client
  -> optional experimental HTTP/3 client
```

The protocol-neutral handler should own:

- Request signal extraction.
- Cache-key construction.
- Store lookup.
- Revalidation and stale-if-error.
- Response safety decisions.
- Store write decisions.
- Observer recording.

Protocol adapters should own:

- Listener setup.
- TLS/QUIC handshake.
- ALPN negotiation.
- Pseudo-header translation.
- Protocol-specific flow control.
- Stream and connection limits.
- Response writing.

## Normalized Request Context

Add protocol metadata without changing cache-key semantics:

```rust
pub struct RequestContext {
    pub downstream_protocol: HttpProtocol,
    pub peer_addr: Option<SocketAddr>,
    pub scheme: String,
    pub authority: String,
}
```

Cache keys should continue using the configured origin scheme and authority, not arbitrary client `Host` values. Protocol metrics can record downstream authority only in bounded/redacted forms if needed later; v0.3.0 should avoid adding host labels.

## Origin Client Facade

`ProxyState` should stop exposing raw `reqwest::Client` directly:

```rust
pub trait OriginClient: Send + Sync {
    async fn send(&self, request: OriginRequest) -> Result<OriginResponse, OriginError>;
}
```

Implementation options:

- `ReqwestOriginClient` for HTTP/1.1 and HTTP/2.
- `ExperimentalH3OriginClient` behind feature/config flags.
- `FallbackOriginClient` that tries the preferred protocol and falls back when configured.

The facade records:

- Attempted upstream protocol.
- Negotiated upstream protocol when known.
- Fallback reason.
- Origin connection errors without exposing sensitive header values.

## Hot-Path Performance Changes

Required changes:

- Pre-index route hints by method/path template at config load time.
- Avoid cloning full header maps more than needed.
- Stream protected and unstoreable responses without full-body buffering.
- Keep body buffering only for candidates that can be fingerprinted or stored.
- Move disk store file reads/writes into `spawn_blocking` or a dedicated store worker.
- Reduce observer lock contention with sharding, per-route atomics, or a bounded event queue.
- Avoid dashboard snapshots holding locks across formatting.
- Add request concurrency limits before allocating large buffers.

Optional changes:

- Metadata-first store reads for stale entries.
- Background eviction for memory and disk stores.
- Static response header templates for debug/cache headers.
- Precomputed route explanation strings for common decision reasons.

## HTTP/2 Runtime

Use Hyper/hyper-util directly where Axum's simple `serve` path lacks configuration.

Required controls:

- ALPN protocols: `h2`, `http/1.1`.
- Optional h2c prior knowledge.
- `max_concurrent_streams`.
- Initial stream and connection windows.
- Keepalive interval and timeout.
- Header list size.
- Graceful shutdown per connection.

HTTP/2 requests must be normalized before policy:

- `:method` -> method.
- `:scheme` -> context scheme.
- `:authority` -> authority/host context.
- `:path` -> URI path/query.
- Hop-by-hop headers are invalid or ignored according to protocol rules.

## HTTP/3 Runtime

Use a QUIC endpoint and HTTP/3 server implementation behind an explicit feature if necessary.

Required controls:

- UDP listener.
- TLS 1.3 certificate and ALPN `h3`.
- QUIC idle timeout.
- Max concurrent bidirectional streams.
- Max field section size.
- QPACK dynamic table bounds.
- Disable 0-RTT for v0.3.0.
- Disable server push.

HTTP/3 requests normalize to the same handler. If the adapter cannot represent a request safely, it rejects that stream and records a protocol error.

## Failure Model

| Failure | Required behavior |
| --- | --- |
| TLS certificate load failure | Fail startup |
| ALPN negotiates unsupported protocol | Close connection with bounded error |
| HTTP/2 malformed pseudo headers | Reject stream/connection per protocol, do not enter cache path |
| HTTP/2 flow-control stall | Timeout stream or connection, emit bounded event |
| HTTP/3 UDP bind failure | Fail startup if enabled |
| HTTP/3 handshake failure | Reject connection, no cache effect |
| HTTP/3 request parse failure | Reject stream, no cache effect |
| Origin HTTP/2 negotiation failure | Fallback if configured, otherwise 502 |
| Origin HTTP/3 failure | Fallback if configured, otherwise 502/504 |
| Store worker saturation | Return origin response and emit store backpressure event |
| Observer queue saturation | Drop low-priority events, keep counters bounded |
| Panic switch active | Disable fresh, revalidated, and stale reuse across all protocols |

## Security Boundaries

- HTTP/2 and HTTP/3 must not permit connection-specific headers to affect cache keys.
- HTTP/3 must disable 0-RTT in v0.3.0 to avoid replay risk for pass-through unsafe methods.
- QUIC amplification protection and validation tokens should use the transport library defaults unless explicitly reviewed.
- `Alt-Svc` must not be advertised for arbitrary Host values unless kubio is configured to serve that authority.
- Protocol metrics must not label arbitrary authority, path, query, or header values.
- TLS private keys are config secrets and must never appear in dashboard APIs or logs.

## Open Questions

- Whether to introduce `kubio-transport` immediately or keep modules under `kubio-proxy` until HTTP/3 grows.
- Whether to use `h3` plus `h3-quinn` directly for downstream HTTP/3 or choose a higher-level adapter if a stable one is available during implementation.
- Whether upstream HTTP/3 should be included in the default binary as a disabled runtime feature or require a separate Cargo feature.
- Whether h2c should support HTTP/1.1 upgrade or prior knowledge only. The design favors prior knowledge only for v0.3.0.
- Whether the benchmark harness should be a Rust crate, shell scripts around external tools, or both. The design favors a Rust harness for CI and optional external tool scripts for operator comparisons.
