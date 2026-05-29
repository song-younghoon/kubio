# Policy and Safety

Status: draft
Target release: `v0.1.0`

## Goals

The policy engine decides whether kubio should protect, observe, store, or reuse a response. It must be deterministic, explainable, conservative, and easy to test.

Policy errors must produce protection or origin pass-through, never unsafe reuse.

## Decision Model

Core decision enum:

```rust
pub enum Decision {
    Reuse,
    StoreOnly,
    ObserveOnly,
    Protect,
    Bypass,
}
```

Required behavior:

- `Reuse`: serve a fresh verified cache entry.
- `StoreOnly`: return origin response and store it for future reuse.
- `ObserveOnly`: return origin response and record safe metadata/fingerprint only.
- `Protect`: mark request/route risky and exclude from reuse.
- `Bypass`: pass through because of config, error, uncertainty, unsupported semantics, or panic switch.

Every decision must include one or more `DecisionReason` values.

## Decision Reasons

Initial reasons:

```rust
pub enum DecisionReason {
    MethodNotCacheable,
    HasAuthorization,
    HasCookie,
    HasSetCookie,
    CacheControlNoStore,
    CacheControlPrivate,
    CacheControlNoCache,
    VaryUnsupported,
    VaryWildcard,
    SensitivePath,
    RangeRequest,
    RequestBodyOnGet,
    StatusNotCacheable,
    ShadowMismatch,
    InsufficientSamples,
    InsufficientShadowValidations,
    LowEstimatedBenefit,
    ObjectTooLarge,
    FingerprintUnavailable,
    PanicSwitchActive,
    PolicyError,
    StoreError,
    ReusableAndFresh,
}
```

User-facing explanations should be derived from these stable reasons, not built ad hoc in the dashboard.

## Hard Deny Rules

Hard deny rules override score and route state:

| Signal | Decision | Reason |
| --- | --- | --- |
| Method not GET/HEAD | Protect | `MethodNotCacheable` |
| `Authorization` request header | Protect | `HasAuthorization` |
| `Cookie` request header | Protect | `HasCookie` |
| `Set-Cookie` response header | Protect or observe only, no store | `HasSetCookie` |
| `Cache-Control: no-store` | Protect or observe only, no store | `CacheControlNoStore` |
| `Cache-Control: private` | Protect or observe only, no store | `CacheControlPrivate` |
| `Cache-Control: no-cache` | Bypass reuse in v0.1.0 | `CacheControlNoCache` |
| `Vary: *` | Bypass reuse | `VaryWildcard` |
| Unsupported `Vary` | Bypass reuse | `VaryUnsupported` |
| Shadow mismatch | Protect/demote | `ShadowMismatch` |
| Panic switch active | Bypass reuse | `PanicSwitchActive` |

Protected does not necessarily mean "do not observe"; it means "do not reuse or store." Safe counters and derived metadata can still be recorded.

## Request Safety Signals

Extract before origin lookup:

```rust
pub struct RequestSignals {
    pub method_cacheable: bool,
    pub has_authorization: bool,
    pub has_cookie: bool,
    pub has_range: bool,
    pub has_body_on_get_or_head: bool,
    pub query_param_count: u16,
    pub sensitive_path_score: u8,
}
```

Do not store raw header values in these signals.

## Response Safety Signals

Extract after origin response headers:

```rust
pub struct ResponseSignals {
    pub status_cacheable: bool,
    pub has_set_cookie: bool,
    pub cache_control: CacheControlClass,
    pub vary: VaryClass,
    pub content_length: Option<u64>,
    pub content_type_class: ContentTypeClass,
}
```

`CacheControlClass`:

- `Absent`
- `Public`
- `Private`
- `NoStore`
- `NoCache`
- `Other`

`VaryClass`:

- `Absent`
- `Supported(Vec<HeaderName>)`
- `Wildcard`
- `Unsupported(Vec<HeaderName>)`

Supported `Vary` headers for v0.1.0:

- `accept`
- `accept-encoding`
- `accept-language`

If support is incomplete, treat the response as not reusable.

## Route Clustering

Route id format:

```text
METHOD /normalized/path
```

Rules:

- Numeric segment -> `{id}`
- UUID-like segment -> `{id}`
- ULID-like segment -> `{id}`
- Long hex segment -> `{id}`
- Empty path -> `/`
- Query string excluded from route id
- If parsing fails, use raw path safely

Examples:

```text
GET /api/products/123 -> GET /api/products/{id}
GET /api/users/018f4df0-3e42-7046-9d81-a061d74a4c18 -> GET /api/users/{id}
GET /api/search?q=phone -> GET /api/search
```

Sensitive path detection should run on raw and normalized path segments.

Sensitive keywords:

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

Sensitive path score should not be a hard deny by itself for all paths, but it should prevent auto promotion unless other evidence is very strong. For v0.1.0, conservative default is to protect sensitive-looking routes.

## Cache Key Generation

Generate only for GET/HEAD requests without precheck hard denies.

Logical components:

```text
method
scheme
authority
path
normalized query
selected Vary request headers
```

Query normalization:

- Parse query into ordered name/value pairs.
- Sort by parameter name.
- Preserve original relative order for repeated names.
- Preserve all parameters.
- Do not drop tracking parameters in v0.1.0.
- Percent-encoding normalization must be deterministic.

Storage and metrics use a hash:

```rust
pub struct CacheKeyHash(String);
```

Raw cache keys must not appear in metrics, logs, or dashboard by default.

## Response Fingerprinting

Fingerprint inputs:

- Status code.
- Stable selected response headers.
- Body hash when body is within `max_fingerprint_body_size`.

Exclude volatile headers:

```text
date
age
server
via
x-request-id
traceparent
tracestate
```

Fingerprint type:

```rust
pub struct ResponseFingerprint {
    pub status: StatusCode,
    pub header_hash: Hash,
    pub body_hash: Option<Hash>,
}
```

Hash recommendation: `blake3` for speed and simplicity.

Fingerprint failure means no auto reuse.

## Route State Machine

Route states:

```rust
pub enum RouteState {
    Watching,
    Candidate,
    ShadowValidated,
    Auto,
    Protected,
}
```

State transitions:

```text
Watching -> Candidate
  repeated safe GET/HEAD traffic, stable fingerprints, meaningful repeat rate

Candidate -> ShadowValidated
  min shadow validations pass with zero mismatches

ShadowValidated -> Auto
  mode is auto, no hard denies, estimated benefit remains meaningful

Any reusable state -> Protected
  hard deny observed for route/key or shadow mismatch

Auto -> Candidate
  confidence expires, route has stale validation, or config becomes stricter
```

For v0.1.0, route-level state is advisory; request/key-level hard denies always win.

## Scoring

Use deterministic scoring for candidate ranking and dashboard explanation. Do not use machine learning.

Suggested score:

```text
base = 0

+30 method GET/HEAD
+20 no Authorization observed
+20 no Cookie observed
+20 no Set-Cookie observed
+20 stable fingerprint
+20 high repeat rate
+10 simple query
+10 origin headers do not forbid storage

-100 no-store
-100 private
-100 Authorization present
-80 Cookie present
-80 Set-Cookie present
-80 unsupported Vary
-60 shadow mismatch
-40 high query cardinality
-30 sensitive path
```

State mapping:

```text
score < 0 -> Protected
0..49 -> Watching
50..79 -> Candidate
80+ plus shadow pass -> Auto eligible
```

Hard denies override score.

## Shadow Validation Policy

Simpler v0.1.0 promotion rule:

```text
A route/key is eligible for auto reuse only after 20 recent shadow validations with 0 mismatches.
```

Additional guards:

- Minimum route samples: 100.
- Minimum key repeats: 5.
- Any mismatch resets or blocks auto eligibility.
- Validation history is bounded and recent.

Auto mode may continue origin sampling. Sampling can be fixed-rate in v0.1.0, for example 1 in 100 eligible hits or at least once per route per minute, but the first release may keep this simple and configurable.

## Freshness Policy

Profiles:

```text
strict: 5s
balanced: 30s
relaxed: 120s
```

Defaults:

- Mode: `watch`
- Freshness: `balanced`
- Debug headers: disabled

Origin headers:

- Respect `no-store`, `private`, `no-cache`.
- `max-age` can cap TTL if lower than profile TTL.
- Do not exceed the selected profile TTL in v0.1.0.

## Explanation Model

Policy APIs should return both machine and display data:

```rust
pub struct PolicyDecision {
    pub decision: Decision,
    pub reasons: Vec<DecisionReason>,
    pub route_state: RouteState,
    pub score: i16,
}
```

Dashboard wording is derived from `DecisionReason`:

- `HasAuthorization`: "Authorization header was observed."
- `HasCookie`: "Cookie header was observed."
- `ShadowMismatch`: "A recent shadow validation saw a different response pattern."
- `ReusableAndFresh`: "A verified fresh response was available."

## Privacy Requirements

The policy engine must not expose:

- Raw `Authorization` values.
- Raw `Cookie` values.
- Raw `Set-Cookie` values.
- Request bodies.
- Raw response bodies for observation.
- Raw query strings in metrics labels.

Policy structs should prefer booleans, counts, classes, hashes, and bounded identifiers.
