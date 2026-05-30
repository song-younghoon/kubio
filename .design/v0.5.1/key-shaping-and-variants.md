# Key Shaping and Variants

Status: implemented
Target release: `v0.5.1`

## Goals

Key shaping is the highest-risk v0.5.1 improvement because it can intentionally
map multiple request shapes to one cache entry. The design must keep it
evidence-gated, explainable, and easy to disable.

v0.5.1 should support:

- query equivalence proof;
- operator-controlled query key compaction;
- slug-like route evidence;
- bounded variant dimensions;
- clear blockers when a route cannot be compacted safely.

## Query Equivalence

### Problem

These requests often produce identical public responses:

```text
GET /notice/1?utm_source=email
GET /notice/1?utm_source=social
GET /notice/1?gclid=abc
```

Without key shaping, kubio stores separate entries and misses more often.

### Evidence Model

For a route and query parameter name, collect:

- parameter name;
- observed value hash count;
- response fingerprint hash;
- cache-key hash before compaction;
- store-safe outcome;
- hard-deny outcome;
- shadow/canary match or mismatch.

Do not store raw values.

### Candidate Rules

A query parameter may become `verified_ignore_candidate` when:

- parameter name is not sensitive;
- route has no hard request deny;
- responses are store-safe;
- at least `min_distinct_values` bounded value hashes were observed;
- at least `min_matching_fingerprints` responses matched across different
  values for the same base key;
- no mismatch was observed in the fresh evidence window;
- the route or exact key is otherwise eligible for reuse.

Suggested defaults:

```yaml
query_equivalence:
  enabled: true
  auto_compact: false
  min_distinct_values: 3
  min_matching_fingerprints: 3
  min_base_keys: 2
  max_mismatches: 0
```

### Sensitive Query Names

These names are blocked from automatic ignore candidates:

```text
token
access_token
auth
authorization
session
sid
jwt
key
api_key
secret
password
signature
sig
state
code
```

The list should be configurable only by adding more names, not by removing
defaults, in v0.5.1.

### Applying Key Compaction

Default behavior:

- kubio reports `verified_ignore_candidate`;
- cache key construction remains unchanged.

Enabled behavior:

```yaml
routes:
  - method: GET
    path: /notice/{id}
    query:
      verified_ignore:
        enabled: true
        allow: ["utm_*", "gclid", "fbclid"]
```

When enabled:

- candidate parameters matching `allow` are ignored in the cache key;
- a compacted-equivalence key hash is recorded;
- canary validation applies to compacted keys;
- mismatch demotes the equivalence group and purges compacted entries.

Global auto compaction can exist, but should default to `false` and should only
apply to a small built-in list of tracking names after proof.

## Query Ordering

Repeated query parameters and ordering must remain deterministic.

Rules:

- normalized order may be stable-sorted only within the existing query-key
  normalization rules;
- repeated values for included parameters remain part of the key;
- ignored parameters are removed only after explicit proof and enablement.

## Slug Path Intelligence

### Problem

Public content endpoints often use slugs:

```text
/articles/summer-release
/blog/how-kubio-decides
/docs/getting-started
```

v0.5.0 primarily focuses on ID-like segments. v0.5.1 should let safe slug
routes collect public-object evidence.

### Slug Candidate Rules

A segment may be slug-like when:

- it is not the first segment;
- it is not a sensitive resource segment;
- it is lowercase alphanumeric plus `-` or `_`;
- it has a bounded length, for example 3 to 96 bytes;
- it is observed in the same route position with enough distinct value hashes;
- static neighboring segments are stable;
- responses are store-safe and fingerprint-stable.

Do not use slug intelligence for:

- `me`;
- `login`;
- `logout`;
- `session`;
- `oauth`;
- user/account/profile/billing/payment/admin routes;
- values containing `@`, `/`, `%2f`, or high-entropy token-like strings.

### Route Template Behavior

Slug intelligence should affect route evidence, not raw cache key identity:

```text
/articles/summer-release -> GET /articles/{slug}
```

The cache key still includes the raw path unless a separate key-shaping proof is
configured and satisfied.

## Variant Dimensions

### Existing Behavior

kubio already supports configured route `vary.allow` names and treats
unsupported `Vary` as unsafe.

### v0.5.1 Refinement

v0.5.1 should make variant evidence visible and bounded:

- configured `Vary` names become explicit variant dimensions;
- snapshots show variant count and top blocker;
- route promotion may require enough evidence per variant;
- unbounded variant cardinality blocks promotion;
- unsupported response `Vary` remains hard protected.

Suggested config:

```yaml
policy:
  adaptive_reuse:
    precision:
      variants:
        max_values_per_dimension: 8
        require_variant_evidence: true
```

## Cookie Traffic

Cookie-bearing requests remain hard protected by default.

v0.5.1 may design but should not ship default cookie variance proof. If
implemented, it must be route-hint-only and require:

- explicit cookie name allowlist;
- `protect_cookies` still enabled globally;
- no `Set-Cookie`;
- public origin headers or strong route confidence;
- canary validation;
- no sensitive cookie names.

This is a candidate for a later release unless implementation scope allows it
without weakening v0.5.1's default safety story.

## Demotion and Purge

Demotion scope depends on the failed proof:

- query equivalence mismatch purges compacted equivalence entries;
- variant mismatch purges entries for the variant key;
- route mismatch purges route entries;
- hard response deny after compaction purges the equivalence group and places it
  in cooldown.

All demotion events use bounded route IDs, parameter names, variant names, and
hashes only.
