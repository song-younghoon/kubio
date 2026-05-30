# Observability and Dashboard

Status: implemented
Target release: `v0.5.0`

Implementation state: route snapshots, dashboard pages, CLI output, debug
headers, and bounded metrics expose adaptive reuse state on `main`.

## Goals

v0.5.0 observability should make adaptive reuse explainable. Users should be
able to answer:

- Why is this route not hitting?
- Is the route hard protected or just waiting for evidence?
- Did a hit come from key validation, public object promotion, origin-public
  headers, or legacy auto?
- What evidence is missing?
- Did kubio demote or purge because safety changed?

## Snapshot Fields

Route snapshots should add bounded fields:

```rust
pub struct RouteSnapshot {
    pub reuse_class: ReuseClass,
    pub reuse_source: Option<ReuseSource>,
    pub path_cardinality: CardinalityClass,
    pub dynamic_path_segments: u8,
    pub distinct_cache_keys_observed: u64,
    pub store_safe_samples: u64,
    pub store_unsafe_samples: u64,
    pub store_safe_rate: f64,
    pub adaptive_blocker: Option<AdaptiveBlocker>,
}
```

Suggested enum values:

```text
ReuseClass:
- hard_protected
- watching
- key_validated
- public_object_candidate
- public_object
- origin_public
- legacy_auto

ReuseSource:
- legacy_auto
- key_validated
- public_object
- origin_public
- revalidated
- stale_if_error

AdaptiveBlocker:
- hard_request_deny
- hard_response_deny
- sensitive_resource
- insufficient_route_samples
- insufficient_distinct_keys
- insufficient_key_evidence
- insufficient_shadow_matches
- shadow_mismatch
- low_store_safe_rate
- origin_not_store_safe
- object_too_large
- panic_switch_active
```

## Dashboard Route List

The route list should include:

- route label;
- current reuse class;
- actual reuse rate;
- estimated potential reuse rate;
- path cardinality;
- distinct key count;
- top blocker;
- hard protected count;
- demotion count.

Sort order should prioritize:

1. `public_object_candidate` routes with high potential savings;
2. routes blocked by a single actionable threshold;
3. protected routes with high traffic;
4. already hitting routes.

## Route Detail

Route detail should show a compact evidence panel:

```text
Reuse class: public_object_candidate
Path cardinality: high
Distinct keys observed: 17
Store-safe samples: 24 / 24
Shadow matches: 3
Shadow mismatches: 0
Next threshold: public_object_min_route_samples 24 / 20 passed
Next threshold: public_object_min_distinct_keys 17 / 3 passed
Next threshold: public_object_min_shadow_matches 3 / 3 passed
```

For protected routes:

```text
Reuse class: hard_protected
Reason: sensitive_resource
Sensitive segment: user
```

The sensitive segment may be shown because it is a static route-template
segment, not a raw identifier.

## Events

Add event types:

```text
adaptive_key_validated
public_object_candidate_detected
public_object_promoted
public_object_demoted
route_entries_purged_after_adaptive_demote
origin_public_fast_path_applied
adaptive_reuse_blocked
```

Events must include bounded reasons and route templates only. Cache key hashes
may be included where existing privacy rules allow them.

## Metrics

Add counters:

```text
kubio_adaptive_reuse_promotions_total{class,reason}
kubio_adaptive_reuse_demotions_total{reason}
kubio_adaptive_reuse_hits_total{source}
kubio_adaptive_reuse_blocks_total{reason}
kubio_public_object_candidates_total
kubio_origin_public_fast_path_total{outcome}
```

Add gauges derived from snapshots:

```text
kubio_routes_by_reuse_class{class}
kubio_routes_by_path_cardinality{cardinality}
```

Keep labels bounded. Do not label metrics by raw path, raw query, authority, or
header values.

## CLI

`kubio routes` should include reuse class and blocker:

```text
GET /notice/{id}  public_object  reuse=42.1%  keys=120  blocker=none
GET /user/{id}    hard_protected  reuse=0.0%   blocker=sensitive_resource
```

`kubio explain "GET /notice/{id}"` should include:

```text
Class: public_object_candidate
Why not reusing yet: insufficient_shadow_matches
Evidence:
- route samples: 18 / 20
- distinct keys: 7 / 3
- store-safe rate: 100%
- shadow mismatches: 0
```

## Debug Headers

When debug headers are enabled, include bounded adaptive state:

```text
x-kubio-cache: hit
x-kubio-reuse-source: public_object
x-kubio-route-state: auto
x-kubio-reuse-class: public_object
```

For misses:

```text
x-kubio-cache: miss
x-kubio-reuse-class: public_object_candidate
x-kubio-blocker: insufficient_shadow_matches
```

Do not include raw route examples, raw cache-key material, or thresholds that
would expose request-specific data.

## Privacy Review

Before release, serialize representative snapshots and metrics from tests that
use sensitive-looking IDs and raw token/cookie values. Assert that outputs do
not contain:

- raw dynamic IDs;
- raw query values;
- Authorization values;
- Cookie values;
- Set-Cookie values;
- validator values;
- response body content.
