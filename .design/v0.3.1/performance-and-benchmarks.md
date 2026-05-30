# Performance and Benchmarks

Status: implemented smoke subset with release budget artifacts
Target release: `v0.3.1`

## Goals

v0.3.1 must not only add HTTP/3; it must measure the new runtime and commit release budgets.

## Dedicated Benchmark Crate

Add:

```text
crates/kubio-bench
```

The crate:

- Start a local origin fixture.
- Start kubio with generated scenario config.
- Warms routes to watch/shadow/auto thresholds.
- Runs fixed-request smoke scenarios.
- Support HTTP/1.1, HTTP/2, and HTTP/3 clients.
- Emits machine-readable JSON.
- Emits a budget pass/fail summary.
- Avoids external tools for the core CI path.

External tools remain optional:

- `curl --http2`.
- `curl --http3`.
- `h2load`.
- `oha`.
- `h3i`.

## Scenarios

Implemented in the dedicated crate:

- HTTP/1.1 fresh memory hit smoke.
- HTTP/2 fresh memory hit smoke.
- HTTP/3 downstream fresh memory hit smoke behind `experimental-http3`.

Still covered by existing shell smoke/integration tests:

- HTTP/1.1 protected request.
- HTTP/1.1 fresh disk hit.
- HTTP/1.1 304 revalidation.
- HTTP/1.1 stale-if-error.
- HTTP/2 multiplexed pass-through.
- HTTP/3 downstream pass-through.
- HTTP/3 protected request.
- HTTP/3 upstream preferred with success.
- HTTP/3 upstream preferred with fallback.
- Large unstoreable response streaming.
- Metrics render under load.

Output fields:

```json
{
  "scenario": "fresh-hit",
  "downstream_protocol": "http3",
  "upstream_protocol": "none",
  "mode": "auto",
  "requests": 50000,
  "concurrency": 128,
  "p50_ms": 1.0,
  "p95_ms": 2.5,
  "p99_ms": 4.0,
  "throughput_rps": 30000,
  "origin_requests": 0,
  "reused_responses": 50000,
  "protected_requests": 0,
  "protocol_fallbacks": 0,
  "budget": "pass"
}
```

## Budget Strategy

Budgets should be relative where machines vary and absolute only where behavior is deterministic.

Initial v0.3.1 release budgets:

| Scenario | Budget |
| --- | --- |
| HTTP/1.1 fresh memory hit | p95 no worse than v0.3.0 baseline by more than 10% on release runner |
| HTTP/2 fresh hit | p95 no worse than v0.3.0 baseline by more than 10% on release runner |
| HTTP/3 fresh hit | Functional and within 25% p95 of HTTP/2 fresh hit on release runner |
| HTTP/3 pass-through | p95 overhead documented and no unbounded buffering |
| Protected large response | No full-body buffering |
| HTTP/3 protected request | Never stored, protected counter increments |
| HTTP/3 upstream fallback | Exactly one fallback event for replayable failure scenario |
| Metrics render under load | Endpoint responds before scenario timeout |

Do not fail CI on normal latency noise in every PR. Required gates:

- Smoke benchmark runs.
- Safety counters are present.
- Protocol counters are present.
- Release workflow enforces budgets on tagged release candidates.

## Performance Risks

- QUIC handshake overhead can dominate small request scenarios.
- UDP buffer sizing can affect high-concurrency results.
- h3 task-per-stream overhead can show up before policy/cache overhead.
- QPACK dynamic tables can increase memory and complexity.
- Upstream HTTP/3 connection pooling must avoid reconnecting on every request.

## Acceptance

- `cargo run -p kubio-bench -- --scenario fresh-hit --protocol h3 --output json` works with `experimental-http3`.
- CI runs at least a smoke subset for h1/h2/h3.
- Release workflow stores benchmark JSON as an artifact.
- Release notes include budget results.
- Benchmark output contains safety counters, not just latency.
