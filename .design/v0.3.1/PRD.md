# PRD: kubio v0.3.1

Document status: design draft
Target release: `v0.3.1`
Core philosophy: **ship HTTP/3 experimentally, prove safety, and measure it**

## 1. Product Summary

kubio v0.3.1 converts v0.3.0's HTTP/3 guarded config into a real experimental runtime:

```text
QUIC downstream listener
HTTP/3 request/response adapters
safe Alt-Svc advertisement
upstream HTTP/3 experiment
release-grade benchmarks
```

The release should let an operator enable HTTP/3 deliberately while preserving v0.2.0 and v0.3.0 cache safety semantics.

## 2. Background

v0.3.0 added stable HTTP/2 and protocol-aware observability, but deferred HTTP/3 runtime work because HTTP/3 requires a separate UDP/QUIC transport path and less mature dependencies than the HTTP/1.1 and HTTP/2 stack.

The v0.3.1 design accepts larger changes:

- Add a transport crate.
- Add HTTP/3 dependencies.
- Split protocol adapters from the policy/cache handler.
- Add feature-specific CI and release artifacts.
- Add a dedicated benchmark crate and budgets.

## 3. Goals

v0.3.1 should:

1. Add an `experimental-http3` Cargo feature.
2. Add a downstream HTTP/3 QUIC listener.
3. Normalize HTTP/3 requests into the same policy/cache/revalidation/store path as HTTP/1.1 and HTTP/2.
4. Stream HTTP/3 responses without forcing full body buffering.
5. Disable 0-RTT.
6. Enforce QUIC, stream, header, QPACK, and body buffering limits.
7. Emit `Alt-Svc` only for configured authorities when HTTP/3 is actually available.
8. Add an upstream HTTP/3 path for HTTPS origins behind explicit build and config gates.
9. Fallback from upstream HTTP/3 only when the request is replayable and fallback is configured.
10. Expose bounded HTTP/3 protocol metrics, dashboard fields, events, CLI output, and debug headers.
11. Add HTTP/3 safety, malformed request, fallback, and cache-key tests.
12. Add interoperability smoke with external HTTP/3 clients where available.
13. Add a dedicated `kubio-bench` crate and v0.3.1 release budgets.

## 4. User Experience

Operators can build or install an HTTP/3-enabled artifact:

```bash
cargo build --release --features experimental-http3
```

They can then configure HTTP/3 explicitly:

```yaml
server:
  listen: "0.0.0.0:8443"
  tls:
    cert: "certs/kubio.pem"
    key: "certs/kubio-key.pem"
  protocols:
    http1: true
    http2: true
  http3:
    enabled: true
    listen: "0.0.0.0:8443"
    advertise: true
    authorities:
      - "localhost:8443"
    alt_svc_ma: "1h"
    max_concurrent_streams: 128
    max_field_section_size: "64KiB"
    idle_timeout: "30s"

origin_protocol:
  preferred: "auto"
  fallback: true
  http3_experimental: false
```

If the binary lacks HTTP/3 support, kubio should fail startup with a clear error and no listeners bound.

## 5. Non-Goals

v0.3.1 will not provide:

- Default-on HTTP/3.
- Production support guarantees equal to HTTP/1.1 and HTTP/2.
- 0-RTT.
- HTTP/3 datagrams.
- WebTransport.
- CONNECT-UDP.
- Server push.
- Automatic certificate issuance.
- A distributed cache or global edge network.
- Cache reuse for unsafe methods or authenticated per-user responses.

## 6. Product Principles

### 6.1 Transport Cannot Relax Policy

HTTP/3 changes how bytes reach kubio. It does not change cache eligibility, protected traffic handling, revalidation, stale-if-error, route hints, query hints, or disk persistence rules.

### 6.2 Fail Before Serving Misconfigured HTTP/3

HTTP/3 config should fail before any listener binds when:

- The build lacks HTTP/3 support.
- TLS certificate or key cannot be loaded.
- `server.http3.advertise` is enabled without a listener.
- `server.http3.authorities` is empty while advertisement is enabled.
- Required limits are zero or unbounded.

### 6.3 Keep HTTP/3 Explicit

HTTP/3 is an experimental runtime in v0.3.1. It should be easy to enable, easy to disable, and easy to identify in logs, metrics, CLI output, and release artifacts.

### 6.4 Fallback Must Be Replay-Safe

Upstream HTTP/3 fallback can retry GET, HEAD, and other replayable requests whose bodies have not been streamed or whose bodies are safely buffered. It must not retry an unsafe or non-replayable body after any bytes may have reached an origin.

## 7. Success Metrics

The release is successful when:

- HTTP/3 downstream safe GET can be observed, shadow-validated, stored, and reused.
- HTTP/3 Authorization and Cookie requests are protected and never stored.
- HTTP/3 response hard-denies behave the same as HTTP/1.1 and HTTP/2.
- Malformed HTTP/3 requests are rejected without entering cache logic.
- `Alt-Svc` is emitted only under explicit, valid authority config.
- Upstream HTTP/3 attempts and fallback are visible and deterministic.
- HTTP/3 metrics use bounded labels and omit QUIC IDs, tokens, authorities, paths, query values, and headers.
- Dedicated benchmark output includes latency, throughput, cache counters, protocol counters, and release budget pass/fail.
- v0.1.0 through v0.3.0 safety tests keep passing.
