# kubio v0.5.2 Release Notes

Status: implemented

v0.5.2 improves real API cache hit rates when origins attach dynamic response
metadata to otherwise stable public responses.

## Added

- Response-header equivalence for volatile metadata such as `date`,
  `x-request-id`, `x-response-id`, `x-correlation-id`, distributed tracing
  headers, and common cloud request IDs.
- Policy-aware response fingerprint normalization with a header policy version.
- Cache-hit header sanitization so one-shot response identifiers are stripped
  from stored hits by default.
- `Age` header support on cache hits.
- Route snapshots, dashboard, CLI, debug headers, events, and metrics for
  response header normalization.
- Bench coverage for public routes with dynamic response metadata.

## Safety

- `Set-Cookie`, `Cache-Control`, `Vary`, validators, representation headers,
  Authorization, Cookie, sensitive paths, panic switch, and mismatch handling
  remain hard safety boundaries.
- Unknown response headers are not automatically ignored unless explicitly
  enabled after evidence.
- Raw response header values are not exposed in snapshots, metrics, events,
  debug headers, CLI output, or disk metadata.

## Compatibility

- Existing v0.5.1 configs continue to load.
- Legacy disk entries without header policy metadata are read conservatively and
  cache-hit volatile stripping still applies.
