# Observability and Dashboard

Status: proposed
Target release: `v0.5.2`

## Goals

v0.5.2 observability should make response-header normalization inspectable.
Operators should be able to answer:

- Which headers are ignored by default?
- Which headers are verified volatile candidates?
- Which headers still block fingerprint equivalence?
- Did kubio strip one-shot metadata from cache-hit responses?
- Was a route demoted because a supposedly volatile header correlated with a
  semantic change?

## Snapshot Fields

Extend route snapshots with bounded header-equivalence fields:

```rust
pub struct HeaderEquivalenceRouteSnapshot {
    pub fingerprint_policy_version: u16,
    pub ignored_response_header_count: u64,
    pub suppressed_on_hit_header_count: u64,
    pub verified_header_ignore_candidates: u64,
    pub header_equivalence_blockers: Vec<HeaderEquivalenceBlocker>,
    pub header_equivalence_cooldown_remaining_seconds: Option<u64>,
}
```

Header candidate snapshots:

```rust
pub struct ResponseHeaderEquivalenceSnapshot {
    pub name: String,
    pub class: HeaderEquivalenceClass,
    pub source: HeaderEquivalenceSource,
    pub distinct_value_count: u64,
    pub matching_without_header_count: u64,
    pub mismatch_count: u64,
    pub operator_enabled: bool,
    pub suppressed_on_hit: bool,
    pub blocker: Option<HeaderEquivalenceBlocker>,
}
```

Allowed classes:

```text
unknown
default_ignored
candidate_volatile
verified_volatile_candidate
ignored
sensitive_blocked
mismatch_cooldown
force_included
```

Allowed sources:

```text
default_policy
route_hint
verified_evidence
global_config
force_include
```

## Dashboard Route List

The route list should add a compact header-equivalence indicator:

```text
GET /notice/{id}  public_object  validated  hit=64.2%  hdr=2 ignored  candidate=0
GET /feed/{id}    watching       probation  hit=0.0%   hdr=x-vendor-execution-id candidate
GET /user/{id}    protected      hard       hit=0.0%   blocker=sensitive_path
```

Sort priority should include:

1. high-traffic routes blocked only by volatile response headers;
2. verified header candidates requiring route enablement;
3. routes in header-equivalence cooldown;
4. routes where a force-included header is causing mismatches.

## Route Detail

Example detail panel:

```text
Response header equivalence:
  x-response-id: default_ignored, suppressed_on_hit=true
  x-correlation-id: default_ignored, suppressed_on_hit=true
  x-vendor-execution-id: verified_volatile_candidate, enabled=false
  content-type: force_included

Next action: enable verified ignore for x-vendor-execution-id on this route.
```

For cooldown:

```text
Response header equivalence:
  x-vendor-execution-id: mismatch_cooldown
  reason: semantic_header_mismatch
  cooldown remaining: 9m 12s
```

## CLI

`kubio routes` should expose concise columns:

```text
GET /notice/{id}  public_object  validated  hit=64.2%  hdr_ignored=2  hdr_candidates=0
GET /feed/{id}    watching       probation  hit=0.0%   hdr_candidate=x-vendor-execution-id
```

`kubio explain "GET /notice/{id}"` should show:

```text
Response headers:
- x-response-id: default_ignored, stripped on hit
- x-correlation-id: default_ignored, stripped on hit
- x-vendor-execution-id: verified candidate, not enabled
- content-type: fingerprinted

Next action: add route response_headers.verified_ignore.allow for
x-vendor-execution-id if this header is known to be non-semantic.
```

## Debug Headers

When debug headers are enabled, add bounded response-header state:

```text
x-kubio-header-shape: normalized
x-kubio-response-headers-ignored: x-response-id,x-correlation-id
x-kubio-response-headers-suppressed: x-response-id,x-correlation-id
```

If no headers were ignored:

```text
x-kubio-header-shape: exact
```

Debug headers must include names only, never values. Header name lists should be
bounded and truncated with a count marker if necessary.

## Metrics

Add bounded metrics:

```text
kubio_response_header_equivalence_candidates_total{class}
kubio_response_header_ignored_total{source}
kubio_response_header_suppressed_on_hit_total{source}
kubio_response_header_equivalence_demotions_total{reason,scope}
kubio_response_header_fingerprint_mismatch_total{reason}
```

Labels must not include raw header values. Header names may be labels only for a
bounded built-in allowlist; otherwise use class/source labels and expose names
in snapshots/CLI instead.

## Events

Add bounded events:

```text
response_header_default_ignored
response_header_candidate_detected
response_header_candidate_verified
response_header_ignore_applied
response_header_ignore_rejected
response_header_suppressed_on_hit
response_header_equivalence_demoted
```

Events may include:

- route ID;
- cache key hash;
- lowercased header name;
- class;
- source;
- bounded reason.

Events must not include header values.

## Privacy Review

Before release, run traffic containing:

```text
X-Response-Id: raw-response-id
X-Correlation-Id: raw-correlation-id
Traceparent: raw-traceparent
X-Vendor-Execution-Id: raw-vendor-id
Authorization: Bearer raw-secret
Cookie: session=raw-cookie
```

Assert that raw values do not appear in:

- dashboard JSON;
- metrics;
- events;
- debug headers;
- CLI output;
- disk metadata.
