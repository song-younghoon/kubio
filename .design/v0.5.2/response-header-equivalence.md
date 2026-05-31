# Response Header Equivalence

Status: implemented
Target release: `v0.5.2`

## Goals

Response-header equivalence should let kubio recognize stable public responses
when only per-request metadata headers change. It must not weaken response
safety checks or representation identity.

The policy answers four questions:

1. Which response headers are part of the representation fingerprint?
2. Which headers are ignored by default because they are non-semantic metadata?
3. Which headers can become verified ignore candidates after evidence?
4. Which ignored headers should be stripped from cache hits?

## Header Taxonomy

### Hard Safety Headers

These headers continue to drive existing policy and cannot be made safe by
fingerprint normalization:

```text
Set-Cookie
Cache-Control
Vary
```

Rules:

- `Set-Cookie` hard-protects by default.
- `Cache-Control: no-store` and `Cache-Control: private` hard-protect.
- `Cache-Control: no-cache` continues to require revalidation.
- `Vary: *` and unsupported `Vary` hard-protect.

### Representation Headers

These headers describe the selected representation and remain included in the
fingerprint by default:

```text
Content-Type
Content-Encoding
Content-Language
Content-Location
Content-Range
Location
Link
```

If they change, kubio should treat the response as different unless a future
release designs a narrower, explicit equivalence rule.

### Validator and Freshness Headers

These headers remain fingerprint-sensitive by default:

```text
ETag
Last-Modified
Expires
Surrogate-Control
```

Rationale:

- validators affect downstream and kubio revalidation behavior;
- freshness headers affect cache semantics;
- changing validators with stable bodies may be common, but validator
  equivalence needs its own design and tests.

`Age` is excluded from the fingerprint because it is cache transit metadata.

### Default Volatile Metadata Headers

These headers are non-semantic metadata and can be excluded from fingerprints by
default:

```text
Date
Age
Server
Via
X-Request-Id
X-Response-Id
X-Correlation-Id
X-Trace-Id
Request-Id
Response-Id
Correlation-Id
Traceparent
Tracestate
X-Amzn-Requestid
X-Amzn-Trace-Id
X-Cloud-Trace-Context
X-B3-Traceid
X-B3-Spanid
X-B3-Parentspanid
X-B3-Sampled
X-B3-Flags
CF-Ray
Fastly-Trace-Id
```

The exact default list should be centralized in `kubio-core` and covered by
tests. Header matching is case-insensitive.

### Sensitive or Business-State Header Names

These names cannot become automatic ignore candidates:

```text
authorization
cookie
set-cookie
proxy-authorization
www-authenticate
x-user-id
x-account-id
x-session-id
x-auth-*
x-token-*
x-api-key
x-csrf-*
x-feature-*
x-permission-*
x-role-*
x-plan-*
x-entitlement-*
```

The list should be conservative and additive. Operators may add more blocked
names globally.

## Fingerprint Normalization

v0.5.2 should replace the implicit `stable_header_hash(headers)` behavior with
a policy-aware normalizer:

```rust
pub struct HeaderFingerprintPolicy {
    pub version: u16,
    pub default_volatile: HeaderNameSet,
    pub verified_ignored: HeaderNameSet,
    pub force_include: HeaderNameSet,
    pub sensitive_blocked: HeaderPatternSet,
}

pub struct HeaderFingerprintResult {
    pub hash: String,
    pub included_names: Vec<String>,
    pub ignored_names: Vec<HeaderIgnoreRecord>,
    pub blocked_names: Vec<HeaderBlockRecord>,
    pub policy_version: u16,
}
```

The response fingerprint should include:

- status;
- normalized header hash;
- body hash;
- header fingerprint policy version.

Adding the policy version prevents silent comparisons between legacy and
v0.5.2 fingerprint semantics.

## Default Ignore Rules

A default volatile header may be ignored only if:

- the request has no hard request deny;
- the response has no hard response deny;
- status is cacheable;
- body fingerprint is available;
- the header is not also in `force_include`.

Ignoring a default volatile header does not mean the response is store-safe.
Store safety is still decided by the existing policy engine.

## Evidence for Verified Candidates

For each route, cache key or equivalence group, and header name, kubio should
track bounded evidence:

```rust
pub struct ResponseHeaderEquivalenceStats {
    pub name: String,
    pub class: HeaderEquivalenceClass,
    pub distinct_value_hashes: u64,
    pub matching_without_header_count: u64,
    pub mismatch_count: u64,
    pub store_safe_count: u64,
    pub hard_deny_count: u64,
    pub operator_enabled: bool,
}
```

Classes:

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

A header may become `verified_volatile_candidate` when:

- the name is not in the sensitive/business-state block list;
- the name is not a hard safety, representation, validator, or freshness
  header;
- at least `min_distinct_values` value hashes were observed;
- at least `min_matching_fingerprints` samples match after excluding that one
  header;
- status and body hash match across those samples;
- included semantic headers match across those samples;
- responses are store-safe;
- no shadow or canary mismatch exists in the fresh window.

Suggested defaults:

```yaml
policy:
  response_header_equivalence:
    enabled: true
    verified_ignore:
      enabled: true
      auto_apply_known_metadata: true
      auto_apply_unknown: false
      min_distinct_values: 3
      min_matching_fingerprints: 3
      max_mismatches: 0
      cooldown: "10m"
```

## Applying Verified Ignore

Default behavior:

- curated default volatile metadata headers are ignored immediately;
- unknown verified candidates are shown in observability but not applied;
- route hints can apply operator-approved, candidate-eligible metadata names by
  name or bounded pattern.

Example:

```yaml
routes:
  - match:
      method: GET
      path: /notice/{id}
    response_headers:
      verified_ignore:
        enabled: true
        allow: ["x-vendor-execution-id"]
      force_include: ["etag"]
      preserve_on_hit: []
```

Rules:

- `force_include` wins over default volatile and verified ignore.
- `preserve_on_hit` affects response replay only, not fingerprinting.
- route hints cannot make hard safety headers ignorable.
- route hints also cannot make validator, freshness, representation, or
  sensitive/business-state headers ignorable.
- a mismatch demotes the header-equivalence group and purges affected entries.

## Demotion and Purge

Demotion triggers:

- candidate header exclusion no longer produces matching status/body/semantic
  headers;
- a hard response deny appears after a header-equivalence promotion;
- canary validation detects a fingerprint mismatch under the normalized
  policy;
- evidence ages out or the route enters cooldown.

Demotion behavior:

- default volatile headers remain globally ignored, but the route can still be
  demoted by status/body/semantic-header mismatch;
- verified candidate demotion affects only the route/header group unless the
  mismatch implies broader route safety risk;
- affected entries are purged when their stored fingerprint policy or
  equivalence group is no longer valid;
- cooldown records route ID, header name, scope, and bounded reason only.

## Compatibility

Existing configs should keep working. If `response_header_equivalence` is
absent, v0.5.2 uses safe defaults.

Setting `policy.response_header_equivalence.enabled: false` should preserve
v0.5.1 fingerprint behavior as closely as possible, including the existing
volatile list.
