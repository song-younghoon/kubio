# HTTP/2 Support

Status: implemented with limited per-connection tuning
Target release: `v0.3.0`

## Goals

HTTP/2 support should be a stable v0.3.0 feature for both client-facing and origin-facing traffic.

Implemented status:

- Downstream HTTP/2 is enabled through Axum/Hyper's HTTP/2 support.
- TLS listener config uses rustls and ALPN values derived from `server.protocols`.
- Explicit h2c prior-knowledge mode is available for local and service-mesh deployments.
- Upstream HTTP/2 is enabled through reqwest, including optional prior knowledge.
- HTTP/2 traffic uses the same policy, cache-key, observer, revalidation, stale-if-error, and store paths as HTTP/1.1.
- HTTP/2 request header-list limits are enforced at the proxy boundary.
- Upstream protocol preference can fail closed when fallback is disabled, and negotiated fallback is recorded.

Deferred status:

- Deeper Hyper per-connection HTTP/2 flow-control tuning.
- Broader HTTP/2 interoperability smoke coverage beyond the workspace integration tests.

kubio should support:

- HTTP/2 over TLS through ALPN for browser and production clients.
- Optional cleartext h2c prior-knowledge mode for local and service-mesh deployments.
- HTTP/2 to origins by ALPN for HTTPS origins.
- HTTP/2 prior knowledge to origins when explicitly configured.
- The same cache, policy, revalidation, stale-if-error, route hint, and query behavior as HTTP/1.1.

## Non-Goals

v0.3.0 HTTP/2 will not support:

- HTTP/2 server push.
- HTTP/1.1 Upgrade to h2c unless it falls out safely from the chosen stack.
- CONNECT tunneling.
- WebSocket over HTTP/2.
- Custom priority scheduling beyond library-supported defaults.
- HTTP/2-specific cache semantics that differ from HTTP/1.1.

## User-Facing Config

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
    keepalive_interval: "30s"
    keepalive_timeout: "10s"
    max_header_list_size: "64KiB"

origin_protocol:
  preferred: "auto"
  fallback: true
  http2_prior_knowledge: false
```

Config rules:

- `server.protocols.http2: true` with `server.tls` enables ALPN `h2`.
- `server.protocols.h2c: true` enables cleartext prior-knowledge HTTP/2 on the TCP listener.
- h2c must be explicit because many HTTP/1.1 clients cannot negotiate cleartext HTTP/2 safely.
- `origin_protocol.http2_prior_knowledge: true` should be allowed only for `http://` origins or for explicit operator intent.
- If `origin_protocol.preferred: http2` and fallback is false, origin negotiation failure returns a gateway error.

## Downstream Runtime

Use a configurable Hyper/hyper-util connection stack instead of only `axum::serve` when HTTP/2 settings are needed.

TLS listener:

```text
TcpListener
  -> rustls acceptor
  -> ALPN selects h2 or http/1.1
  -> hyper connection
  -> tower service
  -> protocol-neutral proxy handler
```

h2c prior-knowledge listener:

```text
TcpListener
  -> HTTP/2 prior-knowledge connection
  -> tower service
  -> protocol-neutral proxy handler
```

The implementation should continue to support the current HTTP/1.1 no-TLS local workflow.

## Request Normalization

HTTP/2 pseudo headers become the normalized request:

- `:method` -> `Method`
- `:scheme` -> request context scheme
- `:authority` -> request context authority
- `:path` -> URI path and query

Rules:

- Missing or duplicate required pseudo headers are protocol errors.
- Connection-specific headers are invalid in HTTP/2 and must not influence cache keys.
- `te: trailers` is the only allowed `TE` value.
- Request trailers are passed through for origin traffic where supported, but do not participate in v0.3.0 cache keys.
- GET/HEAD requests with bodies remain protected.

## Response Handling

Responses are written back using the negotiated downstream protocol.

Storeable responses:

- Must still pass v0.2.0 response policy.
- Must not rely on trailers for safety metadata.
- Must not include `Set-Cookie`, `private`, `no-store`, unsupported `Vary`, or unbounded headers.

Responses with trailers:

- Pass through when protocol stack supports trailers.
- Do not store in v0.3.0 unless a future design models trailer safety.
- Emit an explainable reason such as `TrailersUnsupportedForStorage` if a new reason is added.

Debug headers:

```http
x-kubio-status: hit
x-kubio-status: miss
x-kubio-status: revalidated
x-kubio-status: stale
x-kubio-status: protected
x-kubio-status: bypass
```

Header names are lowercase on HTTP/2 as required by the protocol stack.

## Upstream HTTP/2

The origin client should support:

- ALPN-negotiated HTTP/2 for HTTPS origins.
- Optional prior-knowledge HTTP/2 when configured.
- Fallback to HTTP/1.1 when `origin_protocol.fallback: true`.
- Pool settings tuned for multiplexed connections.

The origin protocol actually used should be recorded when the client stack exposes it.

Conditional revalidation over HTTP/2 should send the same validators:

```http
if-none-match: "<etag>"
if-modified-since: Wed, 21 Oct 2015 07:28:00 GMT
```

The cache path must not distinguish an otherwise identical origin response solely by upstream protocol.

## Flow Control

HTTP/2 multiplexing improves connection reuse but can create memory pressure.

Required limits:

- Max concurrent streams.
- Max header list size.
- Initial stream window.
- Initial connection window.
- Read/write timeouts.
- Keepalive interval and timeout.

Default limits should favor safety and predictable memory over peak throughput. Operators can raise them after benchmarking.

## Observability

Add protocol-aware metrics:

```text
kubio_downstream_connections_total{protocol="http2"}
kubio_downstream_active_streams{protocol="http2"}
kubio_origin_requests_total{upstream_protocol="http2",outcome}
kubio_protocol_errors_total{protocol="http2",side="downstream|upstream",kind}
```

Dashboard route details should show:

- Downstream protocol mix.
- Upstream protocol mix.
- Reuse rate by downstream protocol.
- Origin request count by upstream protocol.

## Failure Model

| Failure | Behavior |
| --- | --- |
| TLS ALPN selects HTTP/1.1 | Serve HTTP/1.1 if enabled |
| TLS ALPN selects h2 | Serve HTTP/2 |
| HTTP/2 disabled but client requires h2 | Close connection or reject request |
| Malformed pseudo headers | Protocol error, no cache effect |
| Header list too large | Reject stream/connection, no cache effect |
| Flow-control timeout | Abort stream and record bounded event |
| Origin HTTP/2 unavailable with fallback | Retry/fallback to HTTP/1.1 |
| Origin HTTP/2 unavailable without fallback | Return 502/504 |

## Acceptance

- HTTP/1.1 current quick-start behavior still works.
- TLS listener negotiates HTTP/2 with an HTTP/2-capable client.
- TLS listener still accepts HTTP/1.1 when enabled.
- h2c prior-knowledge mode works when explicitly enabled.
- Multiplexed HTTP/2 requests share a connection and preserve independent cache decisions.
- HTTP/2 protected requests are never reused.
- HTTP/2 fresh hits, revalidation, stale-if-error, and disk store paths pass integration tests.
- Upstream HTTP/2 can be preferred, required, or used automatically with fallback.
- Protocol metrics use bounded labels and no sensitive values.
