# HTTP/3 Runtime

Status: design draft
Target release: `v0.3.1`

## Goals

Implement HTTP/3 as an experimental but real runtime path:

- Downstream QUIC listener.
- h3 request adapter.
- h3 response writer.
- Safe `Alt-Svc`.
- Upstream HTTP/3 experiment.
- Bounded metrics and tests.

## Downstream Listener

Runtime:

```text
UdpSocket
  -> quinn::Endpoint
  -> incoming QUIC connection
  -> h3::server::Connection
  -> per-request stream task
  -> protocol-neutral proxy handler
```

The HTTP/3 listener runs beside the TCP listener:

```text
TCP :8443 -> HTTP/1.1 and HTTP/2
UDP :8443 -> HTTP/3
```

Startup sequence:

1. Validate config and feature availability.
2. Load TLS cert/key.
3. Build Quinn server config with ALPN `h3`.
4. Apply transport limits.
5. Bind UDP.
6. Start TCP and UDP listeners only after all required listeners are ready.

## Request Normalization

HTTP/3 pseudo headers map into the same normalized request model:

- `:method` -> method.
- `:scheme` -> context scheme.
- `:authority` -> context authority.
- `:path` -> URI path/query.

Rules:

- Missing or duplicate required pseudo headers reject the stream.
- Connection-specific headers reject or ignore according to HTTP/3 rules before policy.
- Request trailers do not participate in cache keys in v0.3.1.
- GET/HEAD bodies remain protected according to existing policy.
- Protocol metadata is recorded separately from cache key material.

## Response Writing

The h3 response writer must support:

- Response headers.
- Streaming bodies.
- Empty body responses.
- Early client disconnect.
- Bounded error events.

Storage remains controlled by the protocol-neutral handler. A response may be stored only after the existing policy and body-buffering gates pass.

## 0-RTT

0-RTT is disabled in v0.3.1.

Reason:

- kubio forwards unsafe methods even though it does not cache them.
- Replay protection cannot be guaranteed across all pass-through traffic yet.
- A future design can consider 0-RTT only for verified cache hits, but that is not part of v0.3.1.

## Limits

Required HTTP/3 limits:

- QUIC idle timeout.
- Max concurrent bidirectional streams.
- Max field section size.
- QPACK dynamic table capacity.
- Max UDP payload size.
- Per-request timeout.
- Global in-flight limiter from v0.3.0.
- Max buffered response size from v0.3.0.

Defaults should be conservative. `qpack_max_table_capacity` should default to `0` unless profiling shows material benefit and no memory risk.

## Alt-Svc

`Alt-Svc` can be emitted only when all are true:

- `server.http3.enabled: true`.
- `server.http3.advertise: true`.
- The request arrived over HTTPS.
- The request authority exactly matches `server.http3.authorities`.
- The configured HTTP/3 listener is active.
- The response is not a dashboard/admin response unless explicitly allowed in a later design.

Example:

```http
alt-svc: h3=":8443"; ma=3600
```

Skip reasons must be bounded:

```text
disabled
not_https
authority_not_allowed
listener_unavailable
internal_response
```

## Upstream HTTP/3

Upstream HTTP/3 remains experimental.

Config:

```yaml
origin_protocol:
  preferred: "http3"
  fallback: true
  http3_experimental: true
```

Behavior:

- Try HTTP/3 only for HTTPS origins.
- Record attempted and final upstream protocol.
- Retry lower protocol only for replayable requests.
- Do not change cache keys based on upstream protocol.
- If required HTTP/3 fails and fallback is false, return a bounded gateway error.

Implementation preference:

1. Direct `h3`/Quinn client in `kubio-transport`.
2. Existing reqwest client for HTTP/1.1 and HTTP/2 fallback.
3. Optional reqwest HTTP/3 implementation only behind a separate switch if direct h3 client controls are insufficient.

## Interoperability

Release candidates should be tested with:

- `curl --http3` when available.
- `h3`/Quinn test client.
- Browser smoke where practical.
- Packet inspection only as a manual diagnostic, not a required CI gate.

## Acceptance

- HTTP/3 listener starts only when explicitly enabled.
- HTTP/3 safe GET can be observed, shadow-validated, stored, and reused.
- HTTP/3 protected traffic is never stored.
- HTTP/3 hard-deny responses are never stored.
- HTTP/3 revalidation and stale-if-error follow existing gates where origin support allows.
- `Alt-Svc` is emitted only under valid authority config.
- HTTP/3 metrics and events are bounded and privacy-safe.
- Disabling HTTP/3 removes the UDP runtime path.
