# Observability

Status: implemented with bounded v0.3.1 labels
Target release: `v0.3.1`

## Goals

HTTP/3 must be visible without leaking transport internals or sensitive request data.

Operators should be able to answer:

- Is downstream HTTP/3 enabled?
- Are clients using HTTP/3?
- Are HTTP/3 requests being reused safely?
- Are QUIC handshakes failing?
- Is `Alt-Svc` being advertised or skipped?
- Did upstream HTTP/3 fallback happen?
- Are HTTP/3 limits rejecting work?

## Metrics

Implemented metrics include:

```text
kubio_downstream_requests_total{protocol}
kubio_upstream_requests_total{protocol}
kubio_http3_connections_total{outcome}
kubio_http3_streams_total{outcome}
kubio_http3_response_write_errors_total{phase}
kubio_alt_svc_advertisements_total{outcome,reason}
kubio_upstream_http3_requests_total{outcome}
kubio_protocol_fallbacks_total
kubio_request_duration_seconds_bucket{route_id,le}
kubio_origin_duration_seconds_bucket{route_id,le}
```

Allowed labels:

- `protocol`: `http1`, `http2`, `http3`.
- `downstream_protocol`: `http1`, `http2`, `http3`.
- `upstream_protocol`: `http1`, `http2`, `http3`, `none`, `unknown`.
- `reason`: `configured_authority`, `http3_disabled`, `advertise_disabled`, `missing_authority`, `authority_not_allowed`, `invalid_value`.
- `outcome`: `accepted`, `handshake_failed`, `malformed_request`, `request_body_rejected`, `advertised`, `skipped`, `attempt`, `success`, `failure`, `fallback`, `required_failure`, `skipped_not_https`, `skipped_non_replayable`.
- `phase`: `headers`, `body`, `finish`.

Forbidden labels:

- Raw authority.
- Host.
- Path.
- Query.
- Header names from arbitrary user input.
- Header values.
- Authorization/Cookie/Set-Cookie values.
- Validator values.
- Certificate paths or contents.
- QUIC connection IDs.
- QUIC tokens.
- Peer IPs.

## Events

Add bounded events:

```text
AltSvcAdvertised
AltSvcSkipped
Http3RuntimeError
UpstreamHttp3Fallback
UpstreamHttp3Failed
```

Events should carry bounded reason enums and optional protocol enum values only.

## Dashboard

Overview additions:

- HTTP/3 enabled state.
- Downstream protocol mix.
- Upstream protocol mix.
- Active HTTP/3 connections.
- Active HTTP/3 streams.
- QUIC handshake failure count.
- `Alt-Svc` advertised/skipped counts.
- HTTP/3 fallback count.

Route detail additions:

- Requests by downstream protocol.
- Origin requests by upstream protocol.
- Reused responses by downstream protocol.
- Latest bounded HTTP/3 event.

## CLI

`kubio doctor`:

- Shows whether the binary has `experimental-http3`.
- Validates HTTP/3 cert/key and UDP bind config.
- Reports configured HTTP/3 authorities.
- Reports whether `Alt-Svc` can be emitted.

`kubio routes`:

```text
protocols=h1:120,h2:980,h3:410 upstream=h2:400,h3:90 reused=700
```

`kubio explain`:

```text
Downstream protocols: http3 38%, http2 58%, http1 4%
Upstream protocols: http3 20%, http2 80%
HTTP/3 fallback: replayable_connect_error 3
Alt-Svc: advertised 200 skipped_authority_not_allowed 12
```

## Debug Headers

When debug headers are enabled:

```http
x-kubio-downstream-protocol: http3
x-kubio-upstream-protocol: http3
x-kubio-protocol-fallback: none
```

Values must be bounded enums only.

## Acceptance

- Metrics render with bounded HTTP/3 labels.
- Dashboard and API show protocol mix without raw authorities.
- CLI distinguishes build support from runtime config.
- Debug headers expose only bounded protocol/status values.
- No sensitive values or QUIC identifiers appear in metrics, logs, dashboard APIs, or benchmark output.
