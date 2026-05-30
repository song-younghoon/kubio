# Evidence Ledger and Decay

Status: design draft
Target release: `v0.5.1`

## Goals

v0.5.0 stores enough counters to classify routes, but those counters are mostly
lifetime-oriented. v0.5.1 should add a bounded evidence ledger so route
confidence reflects recent behavior.

The ledger should answer:

- What positive evidence is fresh?
- What negative evidence happened recently?
- Which equivalence group or variant did the evidence apply to?
- Is the route promoted, in probation, in cooldown, or stale?

## Evidence Scope

Record evidence at four scopes:

```text
route
cache_key
query_equivalence_group
variant_key
```

### Route Evidence

Route evidence includes:

- request count;
- store-safe count;
- store-unsafe count;
- shadow matches;
- shadow mismatches;
- canary matches;
- canary mismatches;
- distinct cache-key count;
- dynamic path value count;
- sensitive path samples.

### Cache-Key Evidence

Cache-key evidence includes:

- observations;
- fresh hits;
- revalidations;
- shadow matches;
- shadow mismatches;
- last safe fingerprint hash;
- last evidence timestamp.

### Query-Equivalence Evidence

Query-equivalence evidence tracks whether a query parameter appears irrelevant
for a route.

It includes:

- query parameter name;
- bounded value hash count;
- matched fingerprint count across values;
- mismatch count;
- affected normalized key hash;
- sensitive-name flag;
- explicit operator enablement state.

### Variant Evidence

Variant evidence tracks bounded request dimensions, such as configured `Vary`
headers.

It includes:

- variant dimension name;
- variant value class or hash;
- variant count;
- store-safe rate per variant;
- mismatch count per variant.

## Window Model

Use bounded rolling windows instead of unbounded history for promotion:

```rust
pub struct EvidenceWindow {
    pub started_at: Instant,
    pub last_updated_at: Instant,
    pub positive: u64,
    pub negative: u64,
    pub store_safe: u64,
    pub store_unsafe: u64,
    pub shadow_matches: u64,
    pub shadow_mismatches: u64,
    pub canary_matches: u64,
    pub canary_mismatches: u64,
}
```

Implementation can use fixed-size ring buffers or bucketed counters:

- default bucket size: 1 minute;
- default window: 30 minutes;
- maximum buckets: bounded by config;
- route count remains bounded by existing observability limits.

## Decay Rules

### Positive Evidence Decay

Positive evidence expires when:

- the fresh window elapsed without enough new samples;
- the route was idle longer than `max_idle_confidence_age`;
- operator changes relevant config.

Decay outcome:

```text
strong -> validated -> probation -> unknown
```

Decay never moves to `hard_protected`.

### Negative Evidence

Negative evidence has immediate effect:

- shadow mismatch;
- canary mismatch;
- hard response deny after promotion;
- unsafe revalidation metadata;
- sensitive path or query signal newly observed.

Outcome:

```text
validated/strong -> cooldown
probation -> cooldown
unknown -> unknown with blocker
```

The affected entries are purged according to scope.

### Cooldown

Cooldown prevents immediate re-promotion after negative evidence.

Suggested defaults:

```yaml
confidence:
  cooldown: "10m"
  max_cooldown: "1h"
  cooldown_backoff: 2.0
```

Repeated negative evidence during cooldown extends the cooldown up to the cap.

## Ledger Privacy

The ledger may store:

- route template;
- cache key hash;
- query parameter names;
- short hashes of query values;
- short hashes of slug/dynamic path values;
- bounded classes and counters.

The ledger must not store:

- raw path IDs;
- raw slugs;
- raw query values;
- Authorization values;
- Cookie values;
- Set-Cookie values;
- validators;
- response bodies.

## Snapshot Shape

Route snapshots should expose:

```rust
pub struct PrecisionEvidenceSnapshot {
    pub confidence_tier: ConfidenceTier,
    pub evidence_window_age_seconds: u64,
    pub cooldown_remaining_seconds: Option<u64>,
    pub fresh_positive_samples: u64,
    pub fresh_negative_samples: u64,
    pub canary_matches: u64,
    pub canary_mismatches: u64,
    pub query_equivalence_groups: u64,
    pub variant_groups: u64,
    pub stale_evidence: bool,
}
```

## Storage and Restart Behavior

v0.5.1 can keep the evidence ledger process-local. Disk cache entries may
survive restart, but precision confidence should restart in `unknown` unless a
future release persists signed evidence metadata.

On restart:

- existing entries remain present;
- adaptive precision serve eligibility starts conservative;
- origin-public entries may still use stored freshness if existing v0.5.0 logic
  allows it;
- route/key precision promotion must rebuild from new observations.
