# kubio v0.5.1 Design Index

Status: design draft
Source: v0.5.0 adaptive reuse implementation
Target release: `v0.5.1`

v0.5.0 made automatic reuse useful for public object routes without weakening
the hard safety model. v0.5.1 should make that goal more precise: fewer routes
should be stuck because of noisy cache keys, stale evidence, or coarse route
classification, but the proxy should still fail closed to origin whenever
evidence is ambiguous.

The release theme is:

```text
Precision adaptive reuse: widen safe reuse by proving equivalence, not by
guessing.
```

## Problem Statement

v0.5.0 gives kubio three practical reuse paths:

- exact-key validation;
- route-level public object promotion;
- origin-public fast path.

That is enough for routes such as:

```text
GET /notice/1
GET /notice/2
GET /notice/1
```

It is still coarse for real API traffic. Common public endpoints often include
tracking query parameters, slugs instead of numeric IDs, bounded `Vary`
dimensions, older evidence that should decay, or mixed safe/unsafe response
patterns that deserve a more specific explanation than "not enough evidence".

v0.5.1 should refine v0.5.0's model rather than replace it. The goal is higher
hit rate from better proof:

- prove when query parameters do not affect the response;
- support slug-like object routes without treating every string segment as
  dynamic;
- keep route confidence fresh through windows, decay, and cooldown;
- split reuse eligibility by variants and evidence tiers;
- explain the next missing proof clearly.

## Design Documents

- [PRD](PRD.md)
  - Product goals, user experience, non-goals, and success metrics.
- [Precision Reuse Policy](precision-reuse-policy.md)
  - Confidence tiers, route/key/variant eligibility, canary validation, and
    demotion rules.
- [Evidence Ledger and Decay](evidence-ledger-and-decay.md)
  - Bounded evidence windows, positive/negative evidence, cooldown, and
    time-based aging.
- [Key Shaping and Variants](key-shaping-and-variants.md)
  - Query equivalence proof, slug path expansion, variant dimensions, and
    operator-controlled key compaction.
- [Observability and Dashboard](observability-dashboard.md)
  - Dashboard/API fields, CLI output, debug headers, metrics, and explanations.
- [Testing and Release](testing-release.md)
  - Unit, integration, privacy, benchmark, and release gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone work breakdown with acceptance checks.

## In Scope

- Add a precision confidence model on top of v0.5.0 reuse classes.
- Add bounded evidence windows and decay so old safe evidence cannot keep a
  route promoted forever.
- Add route/key/variant blockers that explain which specific proof is missing.
- Add query equivalence proof for parameters that do not change fingerprints.
- Keep automatic query key compaction off by default unless the proof and config
  explicitly allow it.
- Add slug-like path intelligence for public content routes with conservative
  safeguards.
- Add variant-aware reuse for bounded dimensions such as configured `Vary`
  headers without allowing unsupported `Vary`.
- Add sampled canary validation for promoted routes so safety can be checked
  while routes are receiving cache hits.
- Add benchmark scenarios for query-noisy public object routes, slug routes,
  and evidence decay/demotion.

## Out of Scope

- Authenticated or per-user response caching.
- Default reuse for cookie-bearing requests.
- Serving the same cache entry across distinct raw paths without a proven key
  equivalence group.
- Automatic ignoring of sensitive query parameters.
- Machine learning classification.
- Distributed route evidence.
- Redis or shared cache coordination.
- GraphQL automatic reuse.

## Cross-Cutting Constraints

- v0.5.0 hard denies remain hard by default: Authorization, Cookie, unsafe
  methods, Range, GET/HEAD bodies, Set-Cookie, no-store, private, unsupported
  `Vary`, `Vary: *`, uncacheable status, missing fingerprint, oversized object,
  panic switch, and shadow mismatch.
- Query equivalence proof may collapse cache keys only within an explicitly
  bounded equivalence group and only after safe fingerprint evidence.
- Sensitive query names such as `token`, `secret`, `session`, `auth`, `jwt`,
  `password`, and `key` cannot be automatically ignored.
- Slug intelligence must never override sensitive resource path protection.
- Evidence decay must degrade to origin pass-through, not stale unsafe reuse.
- Canary validation failures must demote and purge the affected route or
  equivalence group deterministically.
- Observability must expose route templates, hashes, classes, counters, and
  blockers, not raw IDs, raw query values, cookies, authorization values,
  validators, or body content.

## Milestone Map

- M0: Design, terminology, and schema lock.
- M1: Precision confidence model and evidence ledger.
- M2: Query equivalence and key-shaping proof.
- M3: Slug and variant intelligence.
- M4: Adaptive store/hit flow, canary validation, decay, and demotion.
- M5: Dashboard, metrics, CLI, docs, and examples.
- M6: Benchmarks, safety tests, privacy tests, and release hardening.
