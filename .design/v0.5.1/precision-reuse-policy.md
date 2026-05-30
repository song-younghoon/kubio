# Precision Reuse Policy

Status: implemented
Target release: `v0.5.1`

## Goals

The v0.5.1 precision policy should keep v0.5.0's hard safety model, then add a
more granular answer to three questions:

1. Can kubio store this origin response?
2. Can kubio serve an existing entry for this request?
3. Can kubio intentionally map multiple request shapes to the same cache key?

v0.5.0 mostly answers the first two. v0.5.1 adds first-class key-shaping proof
and fresher confidence state.

## Policy Layers

### Layer 0: Hard Protection

Hard protection is unchanged from v0.5.0. Any hard deny blocks store, serve, and
key shaping:

- unsafe methods;
- `Authorization`;
- `Cookie` by default;
- Range;
- GET/HEAD bodies;
- panic switch;
- sensitive path without explicit acknowledgement;
- `Set-Cookie`;
- `Cache-Control: no-store`;
- `Cache-Control: private`;
- `Vary: *`;
- unsupported `Vary`;
- uncacheable status;
- missing fingerprint;
- oversized object;
- shadow or canary mismatch.

### Layer 1: Store Eligibility

Store eligibility means a response can be persisted for the exact request key.
It can come from:

- origin-public fast path;
- exact-key validation;
- public object route confidence;
- explicit route hint;
- legacy auto state.

Store eligibility does not imply query key compaction.

### Layer 2: Serve Eligibility

Serve eligibility means an existing cache entry can satisfy this request.

Allowed sources:

- `exact_key_validated`;
- `origin_public`;
- `public_object_probation`;
- `public_object_strong`;
- `variant_validated`;
- `legacy_auto`.

Every serve decision must reference an exact cache key or a proven equivalence
key. No route-level decision may serve an entry for an object that was never
fetched.

### Layer 3: Key-Shape Eligibility

Key-shape eligibility means kubio can rewrite part of the request identity when
building a cache key.

Examples:

- ignore `utm_source` for one route after proof;
- normalize query parameter ordering;
- include a bounded `Accept-Language` variant;
- classify a slug segment as a dynamic path segment for route evidence only.

Key-shape eligibility has stricter requirements than route promotion because it
can merge previously distinct request keys.

## Confidence Tiers

v0.5.1 should expose confidence tiers in addition to reuse classes:

```text
unknown
probation
validated
strong
cooldown
hard_protected
```

### `unknown`

The route or key has insufficient fresh evidence.

### `probation`

The route has passed minimum evidence but still needs canary validation or a
larger window before lowering origin traffic.

Behavior:

- may store safe responses;
- may serve exact stored entries;
- should keep canary sampling high.

### `validated`

The route/key/equivalence group has fresh positive evidence and zero recent
mismatches.

Behavior:

- may serve according to its reuse class;
- keeps normal canary sampling.

### `strong`

The route has a sustained safe window and high hit potential.

Behavior:

- may use lower canary sampling;
- appears as a high-confidence route in dashboard summaries;
- still demotes immediately on mismatch.

### `cooldown`

Recent negative evidence exists. The route or equivalence group must pass
through origin until cooldown expires and fresh evidence rebuilds confidence.

### `hard_protected`

A hard deny was observed. The route is not eligible for adaptive reuse.

## Eligibility Objects

The runtime should avoid one monolithic boolean. Each decision should return a
structured eligibility object:

```rust
pub struct PrecisionEligibility {
    pub store: EligibilityState,
    pub serve: EligibilityState,
    pub key_shape: EligibilityState,
    pub reuse_class: ReuseClass,
    pub confidence_tier: ConfidenceTier,
    pub source: Option<ReuseSource>,
    pub blockers: Vec<PrecisionBlocker>,
}
```

Suggested blocker groups:

```text
hard_request_deny
hard_response_deny
sensitive_path
sensitive_query_param
insufficient_route_window
insufficient_key_window
insufficient_query_equivalence
insufficient_variant_evidence
stale_evidence
cooldown_active
canary_mismatch
shadow_mismatch
low_store_safe_rate
variant_unbounded
operator_enablement_required
```

## Canary Validation

Promoted routes and query-equivalence groups should occasionally bypass cache
and fetch origin for validation.

Suggested defaults:

```yaml
policy:
  adaptive_reuse:
    precision:
      canary:
        enabled: true
        probation_rate: 0.10
        validated_rate: 0.02
        strong_rate: 0.005
        min_interval: "30s"
```

Rules:

- canary applies only to GET/HEAD requests that would otherwise be served from
  cache;
- canary responses go through the same hard safety checks;
- matching fingerprints refresh evidence;
- mismatches demote and purge the route or equivalence group;
- canary decisions must use deterministic sampling keyed by route and cache key
  hash to avoid unbounded randomness in tests.

## Promotion and Demotion

### Promotion

Promotion requires:

- no hard denies in the fresh evidence window;
- zero mismatches;
- store-safe rate above threshold;
- enough distinct keys or query variants for the class;
- canary success when canary is enabled;
- no active cooldown.

### Demotion

Demotion triggers:

- shadow mismatch;
- canary mismatch;
- unsafe revalidation metadata;
- hard response deny after promotion;
- store-safe rate drops below threshold in the fresh window;
- evidence window expires without refresh.

Demotion behavior:

- exact-key demotion purges that key;
- route demotion purges route entries when mismatch safety is uncertain;
- query-equivalence demotion purges only entries built from that equivalence
  key unless the mismatch implies broader route risk;
- cooldown records bounded reason and expiry time.

## Config Shape

v0.5.1 should extend `policy.adaptive_reuse` without breaking v0.5.0 configs:

```yaml
policy:
  adaptive_reuse:
    precision:
      enabled: true
      confidence:
        fresh_window: "30m"
        min_window_samples: 20
        strong_window_samples: 100
        max_negative_events: 0
        cooldown: "10m"
      canary:
        enabled: true
        probation_rate: 0.10
        validated_rate: 0.02
        strong_rate: 0.005
```

If `precision.enabled` is absent, v0.5.1 should use safe defaults. Setting it to
`false` should preserve v0.5.0 behavior.
