# HTTP/3 Support

Status: guarded config implemented; QUIC runtime deferred
Target release: `v0.3.0`

## Goals

HTTP/3 support should make kubio usable over QUIC while keeping the feature guarded and measurable.

Implemented status:

- HTTP/3 server and origin config fields are parsed.
- Invalid combinations are rejected before listeners bind.
- Enabling downstream HTTP/3 or upstream HTTP/3 in the default v0.3.0 build fails with a clear operator-facing error.
- Example config documents the guarded behavior.

Deferred status:

- QUIC listener, h3 request adapter, h3 response writer, and Alt-Svc emission.
- Upstream HTTP/3 client path and fallback runtime.
- HTTP/3 safety, interoperability, and metrics tests.

v0.3.0 should provide:

- Experimental downstream HTTP/3 listener over QUIC.
- HTTP/3 request normalization into the same policy/cache handler as HTTP/1.1 and HTTP/2.
- Optional `Alt-Svc` advertisement when kubio is configured as an HTTPS edge.
- Experimental upstream HTTP/3 only behind explicit build and config flags.
- Safe fallback to HTTP/2 or HTTP/1.1 where configured.
- Bounded QUIC, stream, header, and QPACK resource limits.

## Non-Goals

v0.3.0 HTTP/3 will not support:

- Default-on HTTP/3.
- QUIC 0-RTT.
- WebTransport.
- HTTP/3 datagrams.
- CONNECT-UDP.
- Server push.
- Custom congestion-control tuning beyond reviewed library defaults.
- Production support guarantees equivalent to HTTP/1.1 and HTTP/2.

## Dependency Direction

Downstream server path candidates:

- `h3` for HTTP/3 semantics.
- `h3-quinn` for Quinn-backed QUIC transport.
- `quinn` for QUIC endpoint configuration.

Upstream client path candidates:

- reqwest HTTP/3 behind its unstable feature and `reqwest_unstable` cfg.
- A small dedicated h3/quinn client only if reqwest does not meet the needed behavior.

The implementation must include a dependency review because HTTP/3 crates move faster than the HTTP/1.1 and HTTP/2 stack.

## User-Facing Config

```yaml
server:
  listen: "0.0.0.0:8443"
  tls:
    cert: "certs/kubio.pem"
    key: "certs/kubio-key.pem"
  http3:
    enabled: true
    listen: "0.0.0.0:8443"
    advertise: true
    alt_svc_ma: "1h"
    max_concurrent_streams: 128
    max_field_section_size: "64KiB"
    idle_timeout: "30s"

origin_protocol:
  preferred: "auto"
  fallback: true
  http3_experimental: false
```

Config rules:

- `server.http3.enabled: true` requires TLS certificate and key.
- `server.http3.listen` defaults to the TLS listener address if unset, using UDP on the same port.
- `server.http3.advertise: true` requires HTTP/3 to be enabled.
- `origin_protocol.preferred: http3` requires `origin_protocol.http3_experimental: true`.
- If the binary is built without HTTP/3 support, HTTP/3 config must fail startup with a clear message.

## Downstream Runtime

Runtime shape:

```text
UDP socket
  -> quinn Endpoint
  -> h3 server connection
  -> stream request adapter
  -> protocol-neutral proxy handler
  -> h3 response writer
```

HTTP/3 should run alongside the TCP listener:

```text
TCP :443 -> HTTP/1.1 and HTTP/2
UDP :443 -> HTTP/3
```

This is safe because TCP and UDP use separate sockets even on the same address and port.

## Request Normalization

HTTP/3 pseudo headers map to the same normalized model as HTTP/2:

- `:method` -> `Method`
- `:scheme` -> request context scheme
- `:authority` -> request context authority
- `:path` -> URI path and query

Rules:

- Malformed pseudo headers reject the stream.
- Connection-specific headers are invalid and must not influence cache keys.
- Request body handling follows existing policy: unsafe methods and GET/HEAD bodies are protected and pass through.
- Request trailers do not participate in cache keys or storage decisions in v0.3.0.

## Alt-Svc Advertisement

When `server.http3.advertise: true`, kubio can add:

```http
alt-svc: h3=":443"; ma=3600
```

Only advertise when all are true:

- The request arrived over a configured HTTPS authority.
- kubio has a valid HTTP/3 listener for that authority and port.
- The response is not an internal dashboard/admin response unless separately configured.
- The operator explicitly enabled advertisement.

Do not advertise HTTP/3 for arbitrary `Host`/`:authority` values when kubio is not configured as the authoritative edge.

## 0-RTT

Disable QUIC 0-RTT in v0.3.0.

Reason:

- kubio passes through unsafe methods to origin even though it does not cache them.
- 0-RTT replay risk is not acceptable until the proxy can enforce replay-safe semantics across all pass-through traffic.

Future support could allow 0-RTT only for cache hits on verified safe GET/HEAD requests, but that needs a separate design.

## Upstream HTTP/3

Upstream HTTP/3 is experimental:

```yaml
origin_protocol:
  preferred: "http3"
  fallback: true
  http3_experimental: true
```

Behavior:

- If enabled and supported by the build, try HTTP/3 first for HTTPS origins.
- If protocol setup, handshake, or request fails and fallback is true, retry with HTTP/2 or HTTP/1.1 according to config.
- Record the attempted and final upstream protocol.
- Do not change cache keys based on upstream protocol.

If reqwest's HTTP/3 feature is used, the build must document the required unstable cfg and CI should include a separate experimental job.

## Resource Limits

Required limits:

- QUIC idle timeout.
- Max concurrent bidirectional streams.
- Max field section size.
- QPACK dynamic table capacity.
- Max request body signal size.
- Max buffered response size.
- Per-connection request timeout.

Defaults should be conservative because UDP and QUIC endpoints can see high connection churn and amplification pressure.

## Observability

Add:

```text
kubio_downstream_connections_total{protocol="http3"}
kubio_downstream_active_streams{protocol="http3"}
kubio_quic_handshake_failures_total{kind}
kubio_protocol_errors_total{protocol="http3",side="downstream|upstream",kind}
kubio_origin_requests_total{upstream_protocol="http3",outcome}
```

Optional transport stats if exposed by Quinn:

- Connection close reason class.
- Retry/validation token count.
- Datagram receive/send totals.
- Path MTU discovery status class.

Do not expose peer IPs, authorities, query values, header values, certificate contents, or token values in metrics.

## Failure Model

| Failure | Behavior |
| --- | --- |
| UDP bind fails | Fail startup if HTTP/3 enabled |
| Certificate load fails | Fail startup |
| QUIC handshake fails | Reject connection, emit bounded counter |
| Malformed HTTP/3 request | Reject stream, no cache effect |
| Header section too large | Reject stream, no cache effect |
| QPACK decoding error | Close connection/stream per library behavior, no cache effect |
| Origin HTTP/3 fails with fallback | Retry lower protocol |
| Origin HTTP/3 fails without fallback | Return gateway error |
| HTTP/3 feature unavailable at build time | Fail startup for HTTP/3 config |

## Security Constraints

- Disable 0-RTT.
- Disable server push.
- Use certificate identities equivalent to HTTPS listener.
- Bound QPACK and header memory.
- Keep Alt-Svc opt-in.
- Keep protocol errors generic in client-visible responses.
- Do not persist QUIC tokens or connection IDs in cache/store data.
- Do not include QUIC connection IDs in normal metrics labels.

## Acceptance

- HTTP/3 listener starts only when explicitly enabled.
- HTTP/3 safe GET can be observed, shadow-validated, stored, and reused.
- HTTP/3 Authorization/Cookie traffic is protected and never stored.
- HTTP/3 stale revalidation and stale-if-error follow the same gates as HTTP/1.1 and HTTP/2 where origin support allows.
- Alt-Svc is emitted only under explicit, valid configuration.
- HTTP/3 metrics are bounded and privacy-safe.
- HTTP/3 tests run behind a separate feature or CI job.
- Disabling HTTP/3 removes the dependency/runtime path from normal operation.
