# v0.3.1 Dependency Review

Status: implemented

## Decision

kubio uses the direct `h3` + `h3-quinn` + Quinn stack for v0.3.1 instead of
reqwest's unstable HTTP/3 client path.

Reasons:

- Downstream HTTP/3 needs direct QUIC server control; reqwest is client-only.
- Upstream fallback needs bounded attempt/fallback telemetry and replay checks.
- reqwest HTTP/3 requires unstable cfg and would add another experimental
  surface on top of the already experimental runtime.

## Pinned Dependencies

- `h3 = 0.0.8`
- `h3-quinn = 0.0.10`
- `quinn = 0.11.9`
- `rustls-pemfile = 2.2.0`
- `webpki-roots = 0.26`

The HTTP/3 dependencies are behind `experimental-http3` where they are runtime
dependencies. Test and benchmark helpers also use the same stack behind the
same feature.

## API Review Notes

- `h3::server::builder()` supports field-section limits and request/response
  stream handling, but does not expose all QPACK dynamic-table controls needed
  to safely enable nonzero dynamic table capacity in v0.3.1.
- `h3-quinn` provides the bridge type used by both server and client paths.
- Quinn endpoint config applies UDP payload limits; Quinn transport config
  applies idle timeout and bidirectional stream limits.
- TLS versions are aligned on rustls 0.23 via tokio-rustls and Quinn's rustls
  integration. HTTP/3 uses TLS 1.3 and ALPN `h3`.

## Supply Chain

`deny.toml` allows the licenses used by the selected HTTP/3 dependency set, so
no audit or license allowlist exception was required for v0.3.1.
