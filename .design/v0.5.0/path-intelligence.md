# Path Intelligence

Status: draft
Target release: `v0.5.0`

## Goals

Path intelligence should identify object-shaped routes without storing or
exposing raw object identifiers. It should help kubio answer:

- Is this route high-cardinality because it contains object IDs?
- Is the resource name public-looking or sensitive-looking?
- Are many distinct keys producing store-safe responses?
- Is route-level evidence strong enough to help new keys?

## Existing Path Behavior

kubio already normalizes ID-like path segments into route templates:

```text
/notice/1      -> /notice/{id}
/api/users/42  -> /api/users/{id}
```

The cache key still uses the raw path, which is correct. Path intelligence
should not change cache-key identity.

## Segment Classes

### Static Segment

Examples:

```text
notice
articles
products
api
v1
```

Static segments are retained in route templates and can contribute to sensitive
resource classification.

### ID-Like Segment

Examples:

```text
123
018f4df0-3e42-7046-9d81-a061d74a4c18
01HX0K4G8H2M2V6WQ0Y7B8J3CK
```

ID-like segments are replaced with `{id}` in route templates. The observer may
store bounded hashes of values for cardinality, not raw values.

### Unknown Dynamic Segment

Examples:

```text
my-slug-title
2026-05-30
release-v1
```

v0.5.0 may keep these as static route segments unless existing normalization
classifies them as IDs. Slug intelligence is deferred unless needed for the
public object benchmark.

## Sensitive Resource Classification

Sensitive resource names remain protected by default:

```text
me
user
users
account
profile
session
login
logout
billing
payment
checkout
admin
token
oauth
```

Matching is segment-exact after percent decoding and lowercasing.

Sensitive examples:

```text
/user/1
/users/1
/account/123
/api/profile/123
```

Non-sensitive public object examples:

```text
/notice/1
/notices/1
/article/1
/articles/1
/product/1
/products/1
/posts/1
```

v0.5.0 should not need a public allowlist for safety. Non-sensitive is not the
same as safe; it only means the route can collect evidence instead of being
hard protected by name.

## Path Evidence Model

Per route, store bounded path evidence:

```rust
pub struct PathEvidence {
    pub route_id: RouteId,
    pub dynamic_segment_count: u8,
    pub distinct_key_count: BoundedDistinctCounter,
    pub path_cardinality: CardinalityClass,
    pub sensitive_resource: bool,
    pub public_object_candidate: bool,
}
```

Per dynamic segment position:

```rust
pub struct PathSegmentEvidence {
    pub position: u8,
    pub template_segment: String,
    pub seen_count: u64,
    pub distinct_value_class: CardinalityClass,
    pub overflowed: bool,
}
```

Cardinality classes:

```text
one
low
medium
high
unknown
```

Suggested thresholds:

```text
one: 1 distinct value
low: 2-4
medium: 5-16
high: 17+ or bounded counter overflow
unknown: no safe sample or sensitive route
```

These classes match the existing query-intelligence style.

## Public Object Candidate Rules

A route may become `public_object_candidate` when:

- method is GET or HEAD;
- route template contains at least one `{id}`;
- no sensitive resource segment is present;
- at least two distinct raw cache keys have been seen;
- store-safe response samples are present;
- no hard request/response deny was observed for the safe sample set;
- no shadow mismatch was observed.

Candidate status does not serve hits by itself. It only explains why kubio is
collecting route-level evidence.

## Privacy Constraints

Path intelligence must not expose:

- raw ID segment values;
- raw slugs if slug intelligence is added later;
- full raw paths with user identifiers;
- query values;
- Authorization, Cookie, Set-Cookie, validators, or body content.

Allowed observer/dashboard output:

```json
{
  "route_id": "GET /notice/{id}",
  "path_cardinality": "high",
  "dynamic_segment_count": 1,
  "sensitive_resource": false,
  "public_object_candidate": true
}
```

Disallowed output:

```json
{
  "example_path": "/notice/123"
}
```

## Examples

### `/notice/{id}`

```text
route template: GET /notice/{id}
sensitive_resource: false
path_cardinality: high
route class: public_object_candidate -> public_object
```

After enough store-safe samples, route-level evidence can open reuse for new
notice IDs.

### `/user/{id}`

```text
route template: GET /user/{id}
sensitive_resource: true
path_cardinality: unknown
route class: hard_protected
```

Cardinality is not enough to override the sensitive resource segment.

### `/api/products/{id}/reviews`

```text
route template: GET /api/products/{id}/reviews
sensitive_resource: false
path_cardinality: medium/high
route class: public_object_candidate if responses are store-safe
```

The raw path remains part of the cache key, so product review lists do not share
entries across products.

## Deferred Work

- Slug normalization for non-numeric content URLs.
- Approximate distinct counters beyond bounded hash sets.
- Cookie variance proof for public endpoints that receive irrelevant cookies.
- Public/sensitive resource dictionaries configurable by operators.

