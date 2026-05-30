# v0.5.1 Implementation Tasks

Status: implemented and verified locally
Target release: `v0.5.1`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.5.1 scope

## Current Implementation Snapshot

v0.5.0 baseline exists:

- adaptive reuse config;
- exact-key validation;
- origin-public fast path;
- public object route classification;
- bounded path evidence;
- hard protection for sensitive routes;
- adaptive dashboard, CLI, debug headers, metrics, and benchmark scenarios.

The v0.5.1 gap is precision. Safe reuse is effective for simple public object
routes, but noisy query parameters, slug routes, stale confidence, bounded
variants, and operator actionability remain too coarse.

## M0: Design and Terminology

Goal: lock v0.5.1 precision semantics before changing runtime behavior.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.5.1 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add precision reuse policy design.
- [x] M0.1.4 Add evidence ledger and decay design.
- [x] M0.1.5 Add key shaping and variants design.
- [x] M0.1.6 Add observability/dashboard design.
- [x] M0.1.7 Add testing and task breakdown.

Acceptance:

- Query equivalence, slug intelligence, evidence decay, canary validation, and
  variant evidence are documented.
- Hard denies are unchanged from v0.5.0.
- Key compaction is explicitly stricter than route promotion.

## M1: Precision Confidence Model

Goal: add confidence tiers and scoped eligibility objects.

- [x] M1.1 Add `ConfidenceTier`.
- [x] M1.2 Add `PrecisionBlocker`.
- [x] M1.3 Add structured `PrecisionEligibility`.
- [x] M1.4 Split store, serve, and key-shape eligibility.
- [x] M1.5 Preserve v0.5.0 behavior when precision is disabled.
- [x] M1.6 Add unit tests for tier transitions.
- [x] M1.7 Add unit tests proving hard denies block all three eligibility
  dimensions.

Acceptance:

- A route can be `public_object` while confidence is `probation`, `validated`,
  or `strong`.
- A route can store and serve without being allowed to compact cache keys.

## M2: Evidence Ledger and Decay

Goal: replace lifetime-only promotion checks with bounded fresh evidence.

- [x] M2.1 Add bounded evidence window structs.
- [x] M2.2 Track route evidence by window.
- [x] M2.3 Track key evidence by window.
- [x] M2.4 Track positive and negative evidence separately.
- [x] M2.5 Implement positive evidence decay.
- [x] M2.6 Implement cooldown with bounded backoff.
- [x] M2.7 Add snapshot fields for evidence age and cooldown.
- [x] M2.8 Add tests for decay, cooldown, and restart behavior.

Acceptance:

- Old evidence cannot keep a route promoted indefinitely.
- Negative evidence immediately blocks precision reuse.
- Restart remains conservative.

## M3: Query Equivalence and Key Shaping

Goal: prove and optionally apply safe query key compaction.

- [x] M3.1 Add query equivalence evidence structs.
- [x] M3.2 Track bounded query value hashes by route and base key.
- [x] M3.3 Add sensitive query-name denylist for auto ignore candidates.
- [x] M3.4 Add `verified_ignore_candidate` state.
- [x] M3.5 Add config for route-enabled verified ignore.
- [x] M3.6 Update cache key building to apply verified ignore only when enabled.
- [x] M3.7 Add compaction demotion and scoped purge.
- [x] M3.8 Add integration tests for query-noisy public object routes.
- [x] M3.9 Add privacy tests for raw query value non-leakage.

Acceptance:

- `utm_source` can become a verified ignore candidate.
- `token` cannot become an automatic ignore candidate.
- Key compaction is disabled by default.
- Enabled compaction improves second-wave hit rate.

## M4: Slug and Variant Intelligence

Goal: expand public object evidence to safe slug routes and bounded variants.

- [x] M4.1 Add slug-like segment classifier.
- [x] M4.2 Track slug value hashes without storing raw values.
- [x] M4.3 Add sensitive-resource override for slug routes.
- [x] M4.4 Add slug public object candidate logic.
- [x] M4.5 Track configured variant dimensions.
- [x] M4.6 Block unbounded variant cardinality.
- [x] M4.7 Keep unsupported `Vary` as hard protection.
- [x] M4.8 Add integration tests for public slug and sensitive slug routes.

Acceptance:

- `/articles/{slug}` can become public object candidate.
- `/users/{slug}` remains protected by default.
- Variant evidence is bounded and visible.

## M5: Canary Validation, Demotion, and Purge

Goal: continuously validate promoted precision decisions.

- [x] M5.1 Add deterministic canary sampler.
- [x] M5.2 Apply canary to promoted routes and compacted query groups.
- [x] M5.3 Record canary match/mismatch evidence.
- [x] M5.4 Demote and purge on canary mismatch.
- [x] M5.5 Add scoped purge for query-equivalence and variant groups.
- [x] M5.6 Add cooldown events.
- [x] M5.7 Add integration tests for canary match and mismatch.

Acceptance:

- Canary mismatch prevents future hits.
- Purge scope matches the failed proof.
- Canary output is bounded and testable.

## M6: Observability, CLI, Docs, and Examples

Goal: make precision behavior actionable.

- [x] M6.1 Extend route snapshots with confidence, blockers, evidence age, and
  cooldown.
- [x] M6.2 Extend query snapshots with equivalence state.
- [x] M6.3 Update dashboard route list and detail.
- [x] M6.4 Update `kubio routes`.
- [x] M6.5 Update `kubio explain`.
- [x] M6.6 Add precision debug headers.
- [x] M6.7 Add precision metrics and events.
- [x] M6.8 Update README, configuration, how-decides, safety model, metrics,
  examples, roadmap, and release notes.

Acceptance:

- Users can identify the next action for query compaction candidates.
- Metrics and snapshots do not expose raw path or query values.

## M7: Benchmarks and Release Hardening

Goal: prove v0.5.1 improves precision without safety regressions.

- [x] M7.1 Add query-noisy public object benchmark.
- [x] M7.2 Add slug public object benchmark.
- [x] M7.3 Add sensitive slug benchmark.
- [x] M7.4 Add evidence decay benchmark.
- [x] M7.5 Add canary mismatch benchmark.
- [x] M7.6 Compare v0.5.1 precision scenarios against v0.5.0 baseline.
- [x] M7.7 Run full workspace tests.
- [x] M7.8 Run HTTP/3 feature tests.
- [x] M7.9 Run privacy regression tests.
- [x] M7.10 Bump workspace version to `0.5.1`.
- [x] M7.11 Add release notes.

Acceptance:

- Query-noisy and slug public routes show materially higher hit rates than
  v0.5.0 when precision features are enabled.
- Protected user and sensitive query tests remain zero-hit and zero-store by
  default.
- Release docs clearly state that key compaction is stricter than route
  promotion.
