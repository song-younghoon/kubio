# Observability and Dashboard

Status: implemented
Target release: `v0.5.1`

## Goals

v0.5.1 observability should explain precision, not just state. Users should be
able to answer:

- Is this route safe but waiting for fresh evidence?
- Is it blocked by route confidence, key confidence, query equivalence, variant
  evidence, or operator enablement?
- Which query parameters are verified ignore candidates?
- Did evidence decay, canary validation, or cooldown change reuse behavior?
- Which routes have the highest safe hit-rate upside?

## Snapshot Fields

Extend route snapshots with:

```rust
pub struct PrecisionRouteSnapshot {
    pub confidence_tier: ConfidenceTier,
    pub precision_blockers: Vec<PrecisionBlocker>,
    pub evidence_window_age_seconds: u64,
    pub stale_evidence: bool,
    pub cooldown_remaining_seconds: Option<u64>,
    pub canary_matches: u64,
    pub canary_mismatches: u64,
    pub query_equivalence_candidates: u64,
    pub query_compacted_groups: u64,
    pub variant_dimensions: u64,
    pub variant_blockers: Vec<PrecisionBlocker>,
    pub estimated_compaction_savings_rate: f64,
}
```

Query parameter snapshots should add:

```rust
pub struct QueryEquivalenceSnapshot {
    pub name: String,
    pub class: QueryEquivalenceClass,
    pub sensitive: bool,
    pub distinct_value_count: u64,
    pub matching_fingerprint_count: u64,
    pub mismatch_count: u64,
    pub operator_enabled: bool,
    pub blocker: Option<PrecisionBlocker>,
}
```

Allowed classes:

```text
unknown
candidate_ignore
verified_ignore_candidate
compacted
sensitive_blocked
mismatch_cooldown
```

## Dashboard Route List

The route list should show:

- route label;
- reuse class;
- confidence tier;
- current hit rate;
- estimated hit-rate upside from query compaction;
- stale evidence flag;
- cooldown remaining;
- top blocker.

Sort order:

1. verified query ignore candidates requiring operator enablement;
2. public object candidates blocked by one threshold;
3. promoted routes with stale evidence;
4. routes in cooldown;
5. hard protected high-traffic routes.

## Route Detail

Route detail should include a precision evidence panel:

```text
Confidence: validated
Evidence window: 23m old, 44 samples
Canary: 7 match, 0 mismatch
Query equivalence:
  utm_source: verified_ignore_candidate, values=5, matches=5, enabled=false
  token: sensitive_blocked
Variant dimensions:
  accept-language: 2 values, bounded
Next action: enable verified ignore for utm_source on this route
```

For stale evidence:

```text
Confidence: probation
Reason: stale_evidence
Next action: kubio needs 8 fresh safe samples or will pass through to origin
```

For cooldown:

```text
Confidence: cooldown
Reason: canary_mismatch
Cooldown remaining: 9m 12s
Affected scope: query_equivalence_group
```

## CLI

`kubio routes` should include compact precision columns:

```text
GET /notice/{id}       public_object  validated  hit=48.2%  upside=12.4%  blocker=operator_enablement_required
GET /articles/{slug}   candidate      probation  hit=0.0%   blocker=insufficient_slug_evidence
GET /user/{id}         protected      hard       hit=0.0%   blocker=sensitive_path
```

`kubio explain "GET /notice/{id}"` should show:

```text
Class: public_object
Confidence: validated
Key shaping:
- utm_source: verified_ignore_candidate, not enabled
- gclid: verified_ignore_candidate, not enabled
- token: sensitive_blocked
Canary:
- matches: 7
- mismatches: 0
Next action: enable route-level verified query ignore for utm_source/gclid
```

## Debug Headers

When debug headers are enabled, include bounded precision state:

```text
x-kubio-reuse-class: public_object
x-kubio-confidence: validated
x-kubio-reuse-source: public_object
x-kubio-precision-blocker: none
x-kubio-key-shape: exact
```

For compacted keys:

```text
x-kubio-key-shape: query_compacted
x-kubio-reuse-source: query_equivalence
```

For blocked routes:

```text
x-kubio-confidence: cooldown
x-kubio-precision-blocker: canary_mismatch
```

Headers must not include raw query values or raw path values.

## Metrics

Add bounded metrics:

```text
kubio_precision_confidence_routes{tier}
kubio_precision_blocked_total{reason}
kubio_precision_canary_total{outcome}
kubio_query_equivalence_candidates_total{class}
kubio_query_compaction_total{outcome}
kubio_precision_demotions_total{scope,reason}
kubio_evidence_decay_total{from_tier,to_tier,reason}
kubio_variant_groups{dimension_class}
```

Labels must remain bounded. Do not label by raw route examples, raw query
values, header values, cookie values, or authority.

## Events

Add bounded events:

```text
precision_confidence_promoted
precision_confidence_decayed
precision_cooldown_started
precision_canary_match
precision_canary_mismatch
query_equivalence_candidate_verified
query_equivalence_compaction_applied
query_equivalence_demoted
slug_route_candidate_detected
variant_unbounded_detected
```

Events may include route ID, parameter name, variant dimension name, cache key
hash, and bounded reason.

## Privacy Review

Before release, serialize snapshots, metrics, events, debug headers, and CLI
JSON output from traffic containing:

```text
/articles/raw-slug-should-not-leak
/notice/1?utm_source=raw-source
/notice/1?token=raw-token
Authorization: Bearer raw-secret
Cookie: session=raw-cookie
ETag: raw-validator
```

Assert none of the raw values appear.
