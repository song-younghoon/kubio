# Architecture Delta

Status: design draft
Target release: `v0.3.1`

## Goals

v0.3.1 introduces a transport boundary large enough for QUIC without rewriting policy, cache, observer, store, or dashboard behavior.

## Workspace Changes

Add:

```text
crates/kubio-transport
crates/kubio-bench
```

Expected responsibilities:

- `kubio-transport`
  - Own TCP/TLS and UDP/QUIC accept loops.
  - Own HTTP/1.1, HTTP/2, and HTTP/3 protocol adapters.
  - Expose normalized downstream requests and response writers.
  - Own origin client facade and upstream protocol fallback.
  - Hide feature-gated HTTP/3 dependencies from policy/cache crates.
- `kubio-proxy`
  - Own the protocol-neutral proxy handler.
  - Own state wiring for config, observer, store, policy, and origin client.
  - Stop depending directly on listener-specific request types where possible.
- `kubio-cli`
  - Validate HTTP/3 feature availability and config.
  - Surface build support in `doctor`.
- `kubio-core`
  - Extend HTTP/3 config with authority allowlist and QPACK/QUIC limit fields.
  - Keep protocol enums and bounded reason enums.
- `kubio-bench`
  - Run repeatable local scenarios against h1, h2, and h3.
  - Emit JSON results and budget status.

## Dependency Additions

Behind `experimental-http3`:

```toml
h3 = "0.0.8"
h3-quinn = "0.0.10"
quinn = { version = "0.11", default-features = false, features = ["runtime-tokio", "rustls-ring"] }
rustls-pemfile = "2"
```

Dev/test:

```toml
rcgen = "0.14"
```

If direct upstream HTTP/3 is not tractable in the first implementation pass, allow this alternate feature:

```toml
reqwest = { version = "0.13", default-features = false, features = ["http2", "http3", "json", "rustls-tls", "stream"] }
```

That alternate path must be isolated because reqwest HTTP/3 requires `--cfg reqwest_unstable` and is explicitly experimental.

## Feature Model

Workspace feature:

```toml
[features]
default = []
experimental-http3 = [
  "kubio-cli/experimental-http3",
  "kubio-proxy/experimental-http3",
  "kubio-transport/experimental-http3",
]
```

Rules:

- Config can always parse HTTP/3 fields.
- Without `experimental-http3`, enabling downstream or upstream HTTP/3 fails startup.
- With `experimental-http3`, HTTP/3 remains disabled by default at runtime.
- Release workflow builds normal and HTTP/3-enabled artifacts.

## Runtime Shape

```text
TCP listener
  -> optional rustls acceptor with ALPN h1/h2
  -> hyper/hyper-util connection
  -> DownstreamRequest
  -> protocol-neutral proxy handler
  -> DownstreamResponder

UDP listener
  -> quinn Endpoint
  -> h3 server connection
  -> h3 stream adapter
  -> DownstreamRequest
  -> protocol-neutral proxy handler
  -> h3 response writer

OriginClient
  -> reqwest HTTP/1.1 and HTTP/2
  -> optional H3OriginClient
  -> fallback coordinator
```

## Transport Interfaces

The exact Rust types can evolve, but the implementation should converge on this boundary:

```rust
pub struct DownstreamRequest {
    pub context: RequestContext,
    pub request: http::Request<DownstreamBody>,
}

pub struct RequestContext {
    pub downstream_protocol: HttpProtocol,
    pub scheme: String,
    pub authority: String,
    pub peer_addr_present: bool,
}

pub trait DownstreamResponder {
    async fn send(self, response: http::Response<ProxyBody>) -> Result<(), TransportError>;
}

pub trait OriginClient {
    async fn execute(&self, request: OriginRequest) -> Result<OriginResponse, OriginError>;
}
```

Important behavior:

- `RequestContext` can include protocol and normalized authority, but policy must not label raw authority in metrics by default.
- `DownstreamBody` and `ProxyBody` must support streaming.
- The proxy handler should not know whether the writer is Hyper or h3.

## Config Additions

Extend `server.http3`:

```yaml
server:
  http3:
    enabled: false
    listen: "0.0.0.0:8443"
    advertise: false
    authorities: []
    alt_svc_ma: "1h"
    max_concurrent_streams: 128
    max_field_section_size: "64KiB"
    qpack_max_table_capacity: 0
    idle_timeout: "30s"
    max_udp_payload_size: 1350
```

Extend `origin_protocol`:

```yaml
origin_protocol:
  preferred: "auto"
  fallback: true
  http2_prior_knowledge: false
  http3_experimental: false
  http3_max_idle_connections: 32
  http3_idle_timeout: "90s"
```

Validation:

- HTTP/3 enabled requires TLS cert/key.
- `advertise: true` requires HTTP/3 enabled and non-empty authorities.
- `origin_protocol.preferred: http3` requires `http3_experimental: true`.
- Limits must be nonzero except `qpack_max_table_capacity`, which may be zero to disable dynamic table use.
- The HTTP/3 listener may share host/port with the HTTPS TCP listener because UDP and TCP sockets are distinct.

## Failure Model

| Failure | Required behavior |
| --- | --- |
| HTTP/3 feature missing | Fail startup if HTTP/3 config is enabled |
| UDP bind failure | Fail startup if downstream HTTP/3 enabled |
| TLS cert/key load failure | Fail startup |
| QUIC handshake failure | Reject connection, increment bounded counter |
| HTTP/3 malformed pseudo headers | Reject stream, no cache effect |
| Header section too large | Reject stream, no cache effect |
| QPACK decode failure | Close stream/connection per h3 behavior, no cache effect |
| Response write failure | End stream, record bounded error, no store mutation beyond completed safe path |
| Upstream HTTP/3 connection failure with replayable fallback | Retry lower protocol if configured |
| Upstream HTTP/3 connection failure without fallback | Return bounded gateway error |
| Upstream HTTP/3 after request body was streamed | Do not retry unless body is known replayable |

## Security Boundaries

- Disable 0-RTT.
- Disable server push.
- Do not enable HTTP/3 datagrams.
- Bound QPACK and header memory.
- Do not store QUIC tokens or connection IDs.
- Do not expose QUIC connection IDs, tokens, peer IPs, raw authorities, or certificate contents in metrics.
- Keep `Alt-Svc` authority allowlist explicit.
- Keep cache semantics protocol-independent.

## Migration

The existing `kubio-proxy` public entry point should remain source-compatible for normal HTTP/1.1 and HTTP/2 users. HTTP/3 support can require new config and feature-enabled builds. Existing v0.3.0 guarded configs should start working only when the binary has `experimental-http3` and required authority/TLS settings.
