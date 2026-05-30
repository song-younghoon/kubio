# Release Notes v0.3.1

Status: implemented scope; release packaging pending.

## Highlights

- Added `experimental-http3` feature across CLI, proxy, transport, tests, and benchmark runner.
- Added downstream HTTP/3 over QUIC with h3/h3-quinn/Quinn and TLS 1.3 ALPN `h3`.
- Added opt-in `Alt-Svc` advertisement for exact configured authorities only.
- Added experimental upstream HTTP/3 for HTTPS origins with replay-safe fallback.
- Added bounded HTTP/3 connection, stream, write-error, Alt-Svc, and upstream attempt/fallback metrics/events.
- Added `crates/kubio-bench` with h1/h2/h3 smoke JSON and budget pass/fail output.
- Release workflow now uploads standard and HTTP/3-experimental Linux artifacts plus benchmark JSON.

## Known Limits

- HTTP/3 remains off by default and requires binaries built with `--features experimental-http3`.
- Downstream HTTP/3 request bodies are bounded by `policy.max_request_body_size` before entering the proxy handler.
- QPACK dynamic table capacity must remain `0` in v0.3.1.
- Upstream HTTP/3 is experimental, HTTPS-only, and uses buffered response bodies for the attempt path.
- Non-replayable upstream fallback is blocked; replayable fallback is limited to GET/HEAD requests without request bodies.

## Benchmark Summary

See [benchmarks-v0.3.1.md](benchmarks-v0.3.1.md).

| Protocol | Requests | Successes | p95 ms | Budget |
| --- | ---: | ---: | ---: | --- |
| h1 | 5 | 5 | 1.71 | pass |
| h2 | 5 | 5 | 2.16 | pass |
| h3 | 5 | 5 | 1.79 | pass |

## Verification

Required local checks:

```bash
cargo fmt --all --check
cargo test --workspace
cargo test --workspace --features experimental-http3
cargo run -p kubio-bench -- --requests 20 --protocol h1 --output json --fail-on-budget
cargo run -p kubio-bench -- --requests 20 --protocol h2 --output json --fail-on-budget
cargo run -p kubio-bench --features experimental-http3 -- --requests 20 --protocol h3 --output json --fail-on-budget
```

External interoperability smoke with `curl --http3` or a browser is environment
dependent and may be skipped on runners without HTTP/3-capable curl/browser
support; the in-repo h3/Quinn integration tests cover the CI path.
