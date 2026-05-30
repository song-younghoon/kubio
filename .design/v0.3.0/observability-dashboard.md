# Observability and Dashboard

Status: implemented for v0.3.0 local scope; advanced dashboard charts deferred
Target release: `v0.3.0`

## Goals

v0.3.0 observability should make performance and protocol behavior visible without exposing sensitive traffic data.

Implementation status: the v0.3.0 codebase now exposes bounded downstream protocol counts, upstream protocol counts, protocol fallback counts/events, backpressure rejection counts/events, live in-flight gauges, store operation counters/latency totals, and observer event-drop counts in snapshots, dashboard JSON APIs, dashboard pages, CLI output, and Prometheus metrics. Advanced dashboard charting and per-protocol reuse breakdowns remain future work. The design below remains the reference for deeper follow-up work.

Operators should be able to answer:

- Which downstream protocols are clients using?
- Which upstream protocols are origins using?
- Are HTTP/2 or HTTP/3 requests being reused safely?
- Is the proxy overloaded?
- Is disk or observer work affecting latency?
- Did protocol fallback happen?

## Snapshot Model Additions

Extend overview snapshots:

```rust
pub struct ProtocolOverviewSnapshot {
    pub downstream_http1_requests: u64,
    pub downstream_http2_requests: u64,
    pub downstream_http3_requests: u64,
    pub upstream_http1_requests: u64,
    pub upstream_http2_requests: u64,
    pub upstream_http3_requests: u64,
    pub protocol_fallbacks: u64,
    pub protocol_errors: u64,
    pub backpressure_rejections: u64,
    pub observer_events_dropped: u64,
}
```

Extend route snapshots:

```rust
pub struct RouteProtocolSnapshot {
    pub downstream_http1_requests: u64,
    pub downstream_http2_requests: u64,
    pub downstream_http3_requests: u64,
    pub reused_by_protocol: BTreeMap<HttpProtocol, u64>,
    pub origin_by_protocol: BTreeMap<HttpProtocol, u64>,
}
```

Keep maps bounded by enum protocol values only.

## Events

Add bounded event types:

```rust
ProtocolFallbackUsed
ProtocolNegotiationFailed
ProtocolRequestRejected
BackpressureRejected
StoreWorkerSaturated
ObserverEventDropped
Http3AltSvcAdvertised
Http3AltSvcSkipped
```

Event messages must not include raw authority, path, query, Authorization, Cookie, Set-Cookie, validator, certificate, QUIC token, or connection ID values.

## Metrics

Add or extend:

```text
kubio_requests_total{downstream_protocol,decision}
kubio_origin_requests_total{upstream_protocol,outcome}
kubio_reused_responses_total{downstream_protocol,status}
kubio_request_duration_seconds_bucket{downstream_protocol,decision,le}
kubio_origin_duration_seconds_bucket{upstream_protocol,outcome,le}
kubio_downstream_connections_total{protocol}
kubio_downstream_active_streams{protocol}
kubio_protocol_errors_total{protocol,side,kind}
kubio_protocol_fallbacks_total{from,to,reason}
kubio_in_flight_requests
kubio_backpressure_rejections_total
kubio_store_operation_duration_seconds_bucket{store,operation,le}
kubio_observer_events_dropped_total{reason}
```

Allowed labels:

- `protocol`: `http1`, `http2`, `http3`
- `downstream_protocol`: `http1`, `http2`, `http3`
- `upstream_protocol`: `http1`, `http2`, `http3`, `unknown`
- `decision`: existing bounded decision values
- `outcome`: `success`, `timeout`, `error`, `fallback`
- `side`: `downstream`, `upstream`
- `kind`: bounded internal protocol error class
- `reason`: bounded internal reason class

Do not add route labels to high-cardinality protocol metrics unless the existing route metric pattern already does so safely.

## Dashboard

The dashboard should add a protocol/performance section to the existing operational view.

Overview fields:

- Downstream protocol mix.
- Upstream protocol mix.
- Reuse rate by protocol.
- Origin fallback count.
- Backpressure rejections.
- In-flight requests.
- Store operation latency summary.
- Observer dropped event count.

Route detail fields:

- Requests by downstream protocol.
- Origin requests by upstream protocol.
- Reused responses by downstream protocol.
- Latest protocol-related event.
- Revalidation and stale behavior unchanged from v0.2.0.

Store page additions:

- Store operation p95 by operation.
- Async disk worker queue depth if using a worker.
- Store worker saturation count.

HTTP/3 page additions can be folded into overview unless the feature grows:

- HTTP/3 enabled/disabled.
- Alt-Svc advertisement enabled/disabled.
- QUIC handshake failure count.

## CLI

### `kubio doctor`

Add checks:

- TLS cert/key loadable when configured.
- HTTP/2 enabled state.
- h2c enabled state.
- HTTP/3 build support when configured.
- HTTP/3 UDP listener reachable locally when running.
- Metrics expose protocol counters.
- Origin protocol preference valid.

Example:

```text
protocol config: ok
tls certificate: ok
http/2 listener: ok
http/3 listener: disabled
origin protocol fallback: ok
benchmark metadata: ok
```

### `kubio routes`

Add compact columns:

```text
protocols=h1:120,h2:980,h3:0 upstream=h2:400 reused=700
```

Keep output single-line and bounded.

### `kubio explain`

Add protocol details:

```text
Downstream protocols: http2 94%, http1 6%
Upstream protocols: http2 100%
Protocol fallback: none observed
```

Do not include raw hostnames by default.

## Debug Headers

When `debug_headers` is enabled:

```http
x-kubio-status: hit
x-kubio-downstream-protocol: http2
x-kubio-upstream-protocol: http2
```

Rules:

- Keep values bounded.
- Do not expose cache key hashes, route hashes, validator values, query values, or origin hostnames.
- Omit upstream protocol on pure cache hits unless the stored metadata records it safely; `none` is acceptable.

## Benchmark Visibility

If benchmark snapshots are stored, keep them local and bounded:

```json
{
  "scenario": "fresh-hit",
  "downstream_protocol": "http2",
  "p95_ms": 2.8,
  "reused_responses": 50000,
  "origin_requests": 0
}
```

Dashboard can show the latest local benchmark file only if the path is configured. Do not auto-discover arbitrary files.

## Privacy Rules

Observability must not expose:

- Authorization values.
- Cookie values.
- Set-Cookie values.
- Raw query values.
- Request bodies.
- Response bodies.
- Validator values by default.
- TLS private key paths if paths are considered sensitive in a future environment.
- QUIC connection IDs, tokens, or client addresses as metric labels.

## Acceptance

- Metrics render with bounded protocol labels.
- Dashboard shows protocol mix without raw authorities or paths beyond existing route IDs.
- CLI doctor validates protocol config.
- Debug headers expose only bounded protocol/status values.
- Protocol fallback and errors are evented.
- Existing v0.2.0 metrics remain available or have documented compatibility mapping.
