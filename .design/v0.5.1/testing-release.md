# Testing and Release Plan

Status: design draft
Target release: `v0.5.1`

## Unit Tests

### Precision Policy

- Hard request denies still block store, serve, and key shaping.
- Hard response denies still block store, serve, and key shaping.
- Confidence tier transitions follow `unknown -> probation -> validated ->
  strong`.
- Positive evidence decays when the fresh window expires.
- Negative evidence moves promoted routes or groups into cooldown.
- Cooldown blocks promotion and serving until fresh evidence rebuilds.
- Canary match refreshes evidence.
- Canary mismatch demotes and purges the correct scope.

### Query Equivalence

- Non-sensitive query names can become verified ignore candidates after matching
  fingerprints.
- Sensitive query names never become verified ignore candidates.
- Query compaction is not applied when `auto_compact` is false.
- Route-level enablement applies only to allowed names.
- Query equivalence mismatch demotes the group and purges compacted entries.
- Raw query values are not stored or emitted.

### Slug Intelligence

- Safe slug-like segments can contribute to public object candidate evidence.
- Sensitive resource routes with slugs remain hard protected.
- Token-like or high-entropy segments do not become slug candidates.
- Raw slugs are represented only as hashes/classes in snapshots.

### Variants

- Configured bounded variants are included in precision evidence.
- Variant count above the configured limit blocks promotion.
- Unsupported response `Vary` remains hard protected.
- Variant mismatch purges only the variant scope unless route safety is
  uncertain.

## Integration Tests

### Query-Noisy Public Object

Origin routes:

```text
/notice/:id
```

Cases:

- `/notice/1?utm_source=a`, `/notice/1?utm_source=b`, and
  `/notice/1?utm_source=c` become verified ignore candidates after matching
  safe fingerprints.
- Without route enablement, cache keys remain un-compacted.
- With route enablement, second-wave requests hit compacted entries.
- A later fingerprint mismatch demotes the query-equivalence group and purges
  compacted entries.

### Sensitive Query

- `/notice/1?token=a` never produces an automatic ignore candidate.
- `/notice/1?signature=a` never produces an automatic ignore candidate.
- Debug headers, snapshots, metrics, and events do not expose raw values.

### Slug Public Object

- `/articles/summer-release` and `/articles/winter-update` can become a slug
  public object candidate.
- Second-wave slug requests can hit after promotion.
- `/users/jane-doe` remains protected by default.

### Evidence Decay

- A promoted route falls from `strong` to `validated`, `probation`, or
  `unknown` as windows expire.
- Decayed routes pass through to origin when required.
- Fresh safe samples restore confidence.

### Canary Validation

- Canary matching responses refresh confidence.
- Canary mismatch demotes and purges.
- Canary sampling is deterministic enough for integration tests.

## Benchmarks

Add `kubio-bench` scenarios:

### Query Noisy Public Object

```text
GET /notice/1?utm_source=1..N
GET /notice/1?utm_source=1..N
```

Expected:

- v0.5.0 baseline misses because keys differ;
- v0.5.1 with route-enabled verified ignore hits after proof;
- no raw query values appear in output.

### Slug Public Object Sweep

```text
GET /articles/slug-1..N
GET /articles/slug-1..N
```

Expected:

- route promotes after slug evidence;
- second wave hits;
- sensitive slug route benchmark remains zero hits.

### Decay and Canary

```text
promote route
advance evidence window
send canary mismatch
send second-wave requests
```

Expected:

- confidence decays or demotes;
- post-demotion requests go to origin;
- entries are purged according to scope.

## Safety Regression Gates

Existing gates must remain green:

- Authorization protected.
- Cookie protected.
- unsafe methods protected.
- Set-Cookie not stored.
- private/no-store not stored.
- no-cache requires validator and revalidation.
- unsupported `Vary` and `Vary: *` not stored.
- shadow mismatch blocks reuse.
- panic switch disables fresh, revalidated, stale, and precision reuse.
- `/user/{id}` protected.
- `/users/{slug}` protected.

## Privacy Gates

Use test values:

```text
/articles/raw-slug-should-not-leak
/notice/1?utm_source=raw-source
/notice/1?token=raw-token
Authorization: Bearer raw-secret-token
Cookie: session=raw-cookie-secret
ETag: raw-validator
```

Assert snapshots, metrics, events, debug headers, CLI output, and disk metadata
do not contain raw secrets or raw identifiers.

## Release Gates

Before release:

- Run `cargo fmt --all --check`.
- Run `cargo check --workspace`.
- Run `cargo test --workspace`.
- Run `cargo test --workspace --features experimental-http3`.
- Run v0.5.0 adaptive benchmarks.
- Run v0.5.1 query-noisy, slug, decay, and canary benchmark scenarios.
- Compare query-noisy and slug hit rates against v0.5.0 baselines.
- Confirm docs and examples describe key compaction enablement and hard-deny
  limits.
- Add release notes for v0.5.1.
