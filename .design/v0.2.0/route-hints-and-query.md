# Route Hints and Query Intelligence

Status: design draft
Target release: `v0.2.0`

## Goals

Route hints let operators express domain knowledge while keeping kubio's default policy conservative. Query intelligence helps users understand cache-key fragmentation and opt into safer key normalization for known noise parameters.

## Route Hint Principles

Hints are not a bypass mechanism for core safety. They can:

- Narrow or tune freshness.
- Enable stale-if-error within caps.
- Ignore specific query parameters for matching routes.
- Restrict or add supported `Vary` request headers when safely implemented.
- Acknowledge sensitive-looking paths that are public by actual request/response signals.
- Force protection for known unsafe routes.

Hints cannot:

- Allow reuse for Authorization or Cookie requests.
- Allow storage of Set-Cookie responses.
- Allow storage of `private` or `no-store` responses.
- Allow unsafe method reuse.
- Override `Vary: *`.
- Ignore shadow mismatches.
- Serve stale while panic switch is active.

## Config Shape

```yaml
routes:
  - name: "public products"
    match:
      method: GET
      path: "/api/products"
    freshness:
      ttl: "60s"
    query:
      include: ["category", "page", "sort"]
      ignore: ["utm_*", "gclid", "fbclid"]
    vary:
      allow: ["accept", "accept-encoding", "accept-language"]
    stale_if_error:
      enabled: true
      max_stale: "5m"
    safety:
      acknowledge_sensitive_path: false
      force_protect: false
```

Fields:

- `name`: optional display name.
- `match.method`: required HTTP method.
- `match.path`: route template or exact path.
- `freshness.ttl`: optional per-route TTL cap.
- `query.include`: optional allowlist of parameters that affect the key.
- `query.ignore`: optional list/globs of parameters to remove from the key.
- `vary.allow`: optional allowed `Vary` request headers.
- `stale_if_error.enabled`: route-level stale permission.
- `stale_if_error.max_stale`: route-level stale window cap.
- `safety.acknowledge_sensitive_path`: allows sensitive path keyword to stop being a route-level hard deny when no personalized signals exist.
- `safety.force_protect`: always protect this route.

Validation:

- A parameter cannot appear in both `include` and `ignore`.
- `include` and `ignore` must not both be empty when `query` is specified.
- Glob syntax supports only `*` suffix for v0.2.0, for example `utm_*`.
- Hints must compile at startup.
- Conflicting route hints fail startup unless ordering is explicit.

## Route Matching

Matching should use the normalized v0.1.0 route template by default:

```text
GET /api/products/{id}
```

The config `path` can be:

- Exact raw path: `/api/products`
- Normalized template: `/api/products/{id}`

The implementation should choose a single internal matcher:

```rust
pub struct RouteHintMatch {
    pub method: Method,
    pub template: RouteTemplate,
}
```

If multiple hints match, choose the most specific path. If specificity ties, fail config validation unless an explicit `priority` field is introduced.

## Query Intelligence

v0.1.0 includes all query parameters in the cache key. v0.2.0 should observe query behavior and report:

- Parameter names seen per route.
- Occurrence rate.
- Value cardinality class.
- Whether a parameter appears correlated with response fingerprint changes.
- Whether ignoring a parameter would merge keys that have matched fingerprints in shadow mode.
- Whether a configured hint is actively changing cache keys.

Observation data should avoid raw values by default:

```rust
pub struct QueryParamObservation {
    pub route_id: RouteId,
    pub name: String,
    pub seen_count: u64,
    pub approximate_value_cardinality: CardinalityClass,
    pub fingerprint_sensitive: bool,
    pub configured_action: QueryParamAction,
}
```

Cardinality classes:

```text
none
one
low
medium
high
unknown
```

Implementation may use approximate counting or bounded hashes of values. Do not store raw values in metrics or dashboard APIs by default.

## Query Key Construction

Default behavior:

```text
include every query parameter exactly as v0.1.0 does
```

With route hint:

1. Parse the query into ordered pairs.
2. Drop parameters matching `ignore`.
3. If `include` is present, keep only included parameters.
4. Preserve repeated parameter relative order.
5. Sort by parameter name as v0.1.0 does.
6. Hash the full key material.

Ignored parameter names should be recorded as metadata, but raw ignored values should not be logged.

## Safe Suggestions

Dashboard and CLI may show suggestions:

```text
GET /api/products

Query suggestions:
- utm_source appears unrelated to response pattern after 48 shadow comparisons.
- gclid appears unrelated to response pattern after 48 shadow comparisons.
```

Suggestions are advisory. `policy.query_intelligence.auto_ignore` defaults to `false`.

If `auto_ignore` is enabled later, it must require:

- Sufficient shadow comparisons.
- Zero mismatches under the proposed ignored-key grouping.
- Low-risk parameter name class.
- No personalized request/response signals.
- Event emission before activation.

For v0.2.0, implement suggestions and explicit hints first.

## Sensitive Query Parameters

Sensitive-looking parameter names must not be displayed with values:

```text
token
access_token
auth
authorization
session
password
secret
key
signature
sig
```

If a sensitive-looking query parameter appears, the default policy should not automatically protect the route solely because of the name, but the dashboard should mark the parameter as sensitive and never suggest ignoring it automatically.

## Events

New event types:

- `route_hint_applied`
- `route_hint_rejected`
- `query_hint_applied`
- `query_hint_rejected`
- `query_param_suggestion_created`

## Acceptance

- A configured ignored query parameter changes the cache key only for matching routes.
- Non-matching routes keep v0.1.0 query behavior.
- Repeated query parameters preserve relative order after hints.
- Sensitive query values do not appear in logs, metrics, dashboard APIs, or events.
- Dashboard can show parameter names, cardinality class, and suggestions.
- Hints cannot override Authorization, Cookie, Set-Cookie, no-store, private, Vary wildcard, or shadow mismatch protection.
