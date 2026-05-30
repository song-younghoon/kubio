# Performance Plan

Status: implemented for v0.3.0 shipped scope; committed benchmark budgets deferred
Target release: `v0.3.0`

## Goals

v0.3.0 should make kubio's performance measurable and improve the proxy hot path without weakening safe reuse.

Implemented status:

- Global in-flight request limiter with bounded 503 rejection on saturation.
- Configurable origin pool idle limits and idle timeout.
- Configurable bounded response buffering limit.
- Early streaming path for unstoreable responses where configured.
- Large protected and oversized storeable response regression tests.
- Store operation counters/latency totals and bounded store saturation events.
- In-flight request gauges and bounded observer event-drop counters.
- Local benchmark smoke script emits JSON latency, cache, and protocol counters and runs in CI.
- HTTP/1.1 baseline scenario smoke covers pass-through, protected, memory/disk hit, 304, stale-if-error, large unstoreable, and metrics-under-load paths.
- Route-hint lookup and vary-name derived state are built once with deterministic first-match behavior.
- Observer state uses a read/write lock; snapshots clone state under read lock and sort/aggregate outside the lock.
- HTTP/2 config is applied through the Hyper connection builder, including stream, window, keepalive, and header-list settings.
- HTTP/2 header-list limit enforcement remains present at the proxy request boundary.

Deferred status:

- Dedicated benchmark crate and committed release budgets.
- Further observer sharding beyond the read/write lock split.

Performance work must answer three questions:

```text
How much latency does kubio add?
How much origin load does kubio remove?
Which protocol and cache path produced the result?
```

## Baseline First

Before optimization work, create a repeatable baseline against the v0.2.0 implementation.

Required scenarios:

- HTTP/1.1 pass-through safe GET.
- HTTP/1.1 protected request with Authorization.
- HTTP/1.1 fresh memory hit.
- HTTP/1.1 fresh disk hit.
- HTTP/1.1 stale entry with 304 revalidation.
- HTTP/1.1 stale-if-error.
- Large unstoreable response streamed from origin.
- Dashboard `/metrics` render while proxy is under load.

Protocol additions:

- HTTP/2 fresh hit with many concurrent streams.
- HTTP/2 pass-through with many concurrent streams.
- HTTP/3 fresh hit when experimental feature is enabled.
- HTTP/3 pass-through when experimental feature is enabled.

## Benchmark Harness

Preferred implementation:

```text
crates/kubio-bench
```

The harness should:

- Start a local configurable origin.
- Start kubio with an explicit config.
- Warm routes until shadow/auto thresholds are met.
- Run fixed-duration or fixed-request scenarios.
- Report latency percentiles, throughput, origin request count, reused count, protected count, stale count, revalidation count, and protocol mix.
- Emit JSON output for CI comparison.
- Avoid external tools for the core CI path.

Optional helper scripts can use external tools such as `wrk`, `oha`, `h2load`, or `curl` when installed, but release gates should not depend on tools that are unavailable in CI.

Example:

```bash
cargo run -p kubio-bench -- \
  --scenario fresh-hit \
  --protocol h2 \
  --requests 50000 \
  --concurrency 128 \
  --body-size 1024 \
  --output json
```

## Initial Budgets

Budgets should be finalized after the baseline is recorded. Initial design targets:

| Scenario | Target |
| --- | --- |
| Fresh memory hit | No p95 regression against v0.2.0 after normalization |
| Pass-through safe GET | p95 overhead documented and lower than v0.2.0 |
| Protected large response | No full-body buffering in kubio |
| Fresh disk hit | p95 overhead documented and not dominated by blocking runtime work |
| 304 revalidation | Local overhead bounded; origin RTT remains dominant |
| HTTP/2 multiplexed hit | Higher connection efficiency than equivalent HTTP/1.1 connection fanout |
| HTTP/3 hit | Functional and measured, but not required to beat HTTP/2 in v0.3.0 |

Do not set absolute CI budgets until benchmark variance is understood. CI can initially fail only on severe regressions, missing safety counters, or benchmark harness failures.

## Hot-Path Findings From v0.2.0

Likely bottlenecks:

- Route hints are scanned linearly.
- Request and response headers are cloned in multiple places.
- Some response bodies are buffered before the proxy knows whether storage is possible.
- Observer state uses one read/write lock for routes, keys, and events; dashboard snapshots do not hold the lock while sorting or aggregating.
- Disk store uses blocking filesystem operations from async methods.
- Store `get` returns full entries even when metadata would be enough to decide stale/revalidation behavior.
- Dashboard snapshots compute latency distributions from per-route VecDeque values under a lock.
- The origin client has minimal explicit pool/protocol tuning.

Each optimization should be tied to a benchmark or contention signal.

## Required Optimizations

### Route Hint Index

Build an index at config load:

```text
(method, normalized_path_template) -> RouteHintConfig
```

Acceptance:

- Matching remains deterministic.
- Duplicate validation still rejects ambiguous hints.
- Hot path no longer scans all route hints.

### Streaming First For Unstoreable Traffic

The proxy should decide as early as possible whether the response can ever be stored:

- Request is unsafe or protected.
- Request has Authorization or Cookie.
- Request has Range.
- Response has `Set-Cookie`, `private`, `no-store`, unsupported `Vary`, non-200 status, or known oversized `Content-Length`.

If storage/fingerprinting is impossible, stream origin body to the client without buffering the full body.

Acceptance:

- Protected large response test proves kubio does not read the full body into memory.
- Safety observations are still recorded.
- Debug headers still work.

### Bounded Buffering For Candidate Traffic

Candidate storeable responses can still be buffered for fingerprinting and storage, but only up to:

```yaml
performance:
  max_buffered_response_size: "2MiB"
```

This should default to the lower of policy fingerprint and storage object limits.

Acceptance:

- Oversized responses switch to stream/pass-through with `ObjectTooLarge` or `FingerprintUnavailable`.
- No partial body is stored.

### Async Disk Store Work

Disk store reads and writes must not block Tokio worker threads.

Implementation options:

- Wrap blocking disk operations in `tokio::task::spawn_blocking`.
- Use a dedicated store worker task with bounded channels.
- Use async file APIs only if they keep code simpler and correct.

Acceptance:

- Disk write benchmark does not show worker-thread stalls.
- Store saturation returns origin response and emits bounded store event.
- Purge operations remain correct.

### Observer Contention Reduction

Implemented v0.3.0 step:

- Replace the single exclusive mutex with a read/write lock.
- Keep mutation paths deterministic for route promotion, demotion, and safety events.
- Clone observer state under a read lock for dashboard/API snapshots, then sort and aggregate outside the lock.

Future scaling options if contention remains high:

- Sharded route/key locks.
- Atomic per-route counters with a bounded event queue.
- A single writer task receiving bounded observation messages.

Required behavior:

- Counters remain accurate enough for route promotion and metrics.
- Shadow mismatch handling remains immediate enough to stop reuse.
- Event overflow drops low-priority events before safety-critical events.

Acceptance:

- Route promotion tests still pass.
- Shadow mismatch blocks reuse deterministically.
- Dashboard snapshot work does not hold the observer lock while sorting or aggregating.

### Origin Pool Tuning

Expose origin pool and timeout settings:

```yaml
performance:
  origin_pool_max_idle_per_host: 32
  origin_pool_idle_timeout: "90s"
```

HTTP/2 should reuse multiplexed connections where possible.

Acceptance:

- Pass-through benchmark reports origin protocol and reuse behavior.
- HTTP/2 origin test proves multiple requests can share one origin connection.

## Backpressure

Add a global in-flight request limiter:

```yaml
performance:
  max_in_flight_requests: 4096
```

Behavior:

- Acquire before heavy body buffering, store lookup, or origin request work.
- If full, return `503 Service Unavailable` with a bounded event.
- Never serve stale or cached responses by skipping policy checks.
- Do not include route, query, or header values in overload messages.

Future route-level limits are out of scope.

## Metrics

Add or extend:

```text
kubio_requests_total{downstream_protocol,decision}
kubio_origin_requests_total{upstream_protocol,outcome}
kubio_reused_responses_total{downstream_protocol,status}
kubio_request_duration_seconds_bucket{downstream_protocol,decision,le}
kubio_origin_duration_seconds_bucket{upstream_protocol,outcome,le}
kubio_in_flight_requests
kubio_backpressure_rejections_total
kubio_store_operation_duration_seconds_bucket{store,operation,le}
kubio_observer_events_dropped_total{reason}
```

Labels must be bounded:

- `downstream_protocol`: `http1`, `http2`, `http3`
- `upstream_protocol`: `http1`, `http2`, `http3`, `unknown`
- `decision`: existing bounded decision values
- `outcome`: `success`, `timeout`, `error`, `fallback`

## Benchmark Output

JSON example:

```json
{
  "scenario": "fresh-hit",
  "downstream_protocol": "http2",
  "upstream_protocol": "http2",
  "requests": 50000,
  "concurrency": 128,
  "p50_ms": 1.2,
  "p95_ms": 2.8,
  "p99_ms": 4.1,
  "throughput_rps": 18100.0,
  "origin_requests": 0,
  "reused_responses": 50000,
  "protected_requests": 0,
  "revalidation_attempts": 0,
  "stale_served": 0
}
```

## Acceptance

- Benchmark harness exists and runs from a clean checkout.
- Benchmarks report cache behavior and protocol behavior together.
- Hot-path changes are covered by tests and benchmark comparisons.
- Protected and unstoreable responses stream without full buffering.
- Disk I/O does not block runtime worker threads.
- Backpressure is bounded, observable, and does not relax policy.
- Metrics labels remain bounded and free of sensitive values.
