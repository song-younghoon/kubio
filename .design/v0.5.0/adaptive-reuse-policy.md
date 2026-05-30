# Adaptive Reuse Policy

Status: implemented
Target release: `v0.5.0`

Implementation state: adaptive config, hard/evidence split, key validation,
public-object promotion, origin-public fast path, and mismatch purge behavior
are implemented on `main`.

## Goals

The adaptive reuse policy should increase useful hits while keeping the safety
model inspectable. The policy must separate:

- signals that always protect;
- signals that require evidence;
- evidence that can promote a route or key;
- events that demote and purge.

## Current Bottleneck

The current request decision treats any request-level risk reason as immediate
protection. The current reuse path also requires both the route and the exact
cache key to be auto eligible before serving or storing as an auto route.

That means a high-cardinality route such as `/notice/{id}` must see many repeats
of the same raw path before a hit is possible, even when route-level behavior is
consistently public and store-safe.

## Policy Taxonomy

### Hard Deny

Hard-denied traffic is protected or bypassed immediately and cannot contribute
to reuse promotion except as negative evidence.

Request hard denies:

- method is not GET or HEAD;
- `Authorization` header is present and `protect_authorization` is enabled;
- `Cookie` header is present and `protect_cookies` is enabled;
- Range request;
- GET/HEAD request body;
- panic switch active;
- configured route hint `force_protect`;
- sensitive resource path unless explicitly acknowledged by route hint.

Response hard denies:

- `Set-Cookie` when `protect_set_cookie` is enabled;
- `Cache-Control: no-store`;
- `Cache-Control: private`;
- `Vary: *`;
- unsupported `Vary`;
- non-200 status for automatic reuse;
- missing response fingerprint;
- object larger than configured storage or fingerprint limits.

Validation hard denies:

- shadow mismatch;
- unsafe 304 metadata during revalidation;
- corrupt stored entry;
- route or key evidence overflow that makes a safe decision impossible.

### Evidence-Gated Signals

These signals should not automatically protect, but they influence route class
and thresholds:

- dynamic ID-like path segment;
- high path cardinality;
- query parameter count;
- absent origin cache headers;
- route has not yet met store-safe sample thresholds;
- exact key has not yet repeated;
- route has only one observed key and cannot be classified as public object.

## Reuse Classes

### `hard_protected`

The route or request hit a hard deny. It cannot store or reuse.

### `watching`

The default class. kubio observes route, key, response, and fingerprint evidence
but does not serve hits unless an exact key or origin-public fast path qualifies.

### `key_validated`

An exact cache key has repeated enough with matching fingerprints and zero
mismatches. kubio may serve that key even if the route is not yet globally
promoted.

Suggested default:

```text
min_key_observations = 2
min_key_shadow_matches = 1
max_key_shadow_mismatches = 0
```

This makes a stable exact URL eligible for reuse on the third request under
default settings.

### `public_object_candidate`

The route template looks like a public object collection but does not yet have
enough evidence to open route-level reuse.

Candidate signals:

- one or more ID-like path segments;
- non-sensitive resource segment before the ID;
- at least two distinct raw path values;
- store-safe response rate above threshold;
- no hard denies;
- no shadow mismatches.

### `public_object`

The route has enough route-level evidence to let new keys benefit from route
confidence. Once a route is public object:

- a new key may be stored after its first safe origin response;
- that key may hit on the next fresh request;
- exact-key shadow evidence is still tracked;
- any key mismatch demotes the route.

Suggested defaults:

```text
public_object_min_route_samples = 20
public_object_min_distinct_keys = 3
public_object_min_store_safe_rate = 0.98
public_object_min_shadow_matches = 3
public_object_max_shadow_mismatches = 0
```

### `origin_public`

The origin explicitly marks the response as public and cacheable.

Qualifying response headers:

```text
Cache-Control: public, max-age=N
Cache-Control: s-maxage=N
```

Requirements:

- request has no hard deny;
- response has no hard deny;
- fingerprint is available;
- object is within configured size limits;
- TTL is bounded by kubio freshness caps.

Behavior:

- first safe origin response can be stored;
- second fresh request for the same key can hit;
- route evidence still accumulates normally.

## Store and Hit Flow

### Request Flow

1. Build request signals and route ID.
2. Apply hard request denies.
3. Build cache key for cacheable methods.
4. Check route/key eligibility:
   - exact key validated;
   - route public object and key has stored safe entry;
   - origin-public stored entry still fresh;
   - legacy auto route eligibility.
5. Serve fresh hit only if an entry exists for the exact cache key.
6. Fall through to origin otherwise.

### Response Flow

1. Build response signals.
2. Apply hard response denies.
3. Fingerprint safe bounded bodies.
4. Record route evidence, path evidence, key evidence, and shadow comparisons.
5. Store if one of these is true:
   - route is public object;
   - exact key is key validated;
   - origin response is origin public;
   - legacy route auto eligibility passes.
6. Return origin response.

## Demotion and Purge

Demotion triggers:

- any shadow mismatch on a route promoted through `public_object`;
- any hard response deny after prior route promotion;
- unsafe revalidation metadata;
- configured panic switch.

Demotion behavior:

- route class becomes `hard_protected` for shadow mismatch, or `watching` for
  temporary evidence loss when no unsafe data was served;
- all entries for the route should be purged when mismatch or unsafe response
  metadata is observed;
- events should include route ID, bounded reason, and cache key hash when
  available;
- no raw path segment values should be emitted.

## Config Shape

The exact schema can evolve during implementation, but v0.5.0 should avoid
making every threshold a top-level field. A nested policy shape is preferred:

```yaml
policy:
  adaptive_reuse:
    enabled: true
    key_validation:
      min_observations: 2
      min_shadow_matches: 1
      max_shadow_mismatches: 0
    public_object:
      enabled: true
      min_route_samples: 20
      min_distinct_keys: 3
      min_store_safe_rate: 0.98
      min_shadow_matches: 3
      max_shadow_mismatches: 0
    origin_public_fast_path:
      enabled: true
      max_ttl: "5m"
```

Existing v0.4.1 policy fields remain accepted. If both legacy and adaptive
thresholds exist, adaptive thresholds should control the new classes while
legacy thresholds continue to control the existing `Auto` state.

## Route Hints

Existing route hints remain valid.

New optional hint fields may be added:

```yaml
routes:
  - match:
      method: GET
      path: "/notice/{id}"
    safety:
      public_object: true
```

Hint behavior:

- `public_object: true` can lower evidence thresholds but cannot bypass hard
  denies.
- `acknowledge_sensitive_path` may allow a sensitive-looking route to be
  observed as a candidate, but only when Authorization, Cookie, Set-Cookie,
  private, no-store, unsupported Vary, and mismatch checks pass.
- `force_protect` always wins.

## Compatibility

Default mode remains `watch`.

In `watch` and `shadow`, kubio records adaptive evidence but does not serve
fresh hits from new adaptive classes. In `auto`, adaptive classes can serve hits.

Debug headers and dashboard state should make it clear whether a hit came from:

```text
legacy_auto
key_validated
public_object
origin_public
revalidated
stale
```
