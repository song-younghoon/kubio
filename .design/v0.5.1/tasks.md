# v0.5.1 Implementation Tasks

Status: design draft
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

- [ ] M1.1 Add `ConfidenceTier`.
- [ ] M1.2 Add `PrecisionBlocker`.
- [ ] M1.3 Add structured `PrecisionEligibility`.
- [ ] M1.4 Split store, serve, and key-shape eligibility.
- [ ] M1.5 Preserve v0.5.0 behavior when precision is disabled.
- [ ] M1.6 Add unit tests for tier transitions.
- [ ] M1.7 Add unit tests proving hard denies block all three eligibility
  dimensions.

Acceptance:

- A route can be `public_object` while confidence is `probation`, `validated`,
  or `strong`.
- A route can store and serve without being allowed to compact cache keys.

## M2: Evidence Ledger and Decay

Goal: replace lifetime-only promotion checks with bounded fresh evidence.

- [ ] M2.1 Add bounded evidence window structs.
- [ ] M2.2 Track route evidence by window.
- [ ] M2.3 Track key evidence by window.
- [ ] M2.4 Track positive and negative evidence separately.
- [ ] M2.5 Implement positive evidence decay.
- [ ] M2.6 Implement cooldown with bounded backoff.
- [ ] M2.7 Add snapshot fields for evidence age and cooldown.
- [ ] M2.8 Add tests for decay, cooldown, and restart behavior.

Acceptance:

- Old evidence cannot keep a route promoted indefinitely.
- Negative evidence immediately blocks precision reuse.
- Restart remains conservative.

## M3: Query Equivalence and Key Shaping

Goal: prove and optionally apply safe query key compaction.

- [ ] M3.1 Add query equivalence evidence structs.
- [ ] M3.2 Track bounded query value hashes by route and base key.
- [ ] M3.3 Add sensitive query-name denylist for auto ignore candidates.
- [ ] M3.4 Add `verified_ignore_candidate` state.
- [ ] M3.5 Add config for route-enabled verified ignore.
- [ ] M3.6 Update cache key building to apply verified ignore only when enabled.
- [ ] M3.7 Add compaction demotion and scoped purge.
- [ ] M3.8 Add integration tests for query-noisy public object routes.
- [ ] M3.9 Add privacy tests for raw query value non-leakage.

Acceptance:

- `utm_source` can become a verified ignore candidate.
- `token` cannot become an automatic ignore candidate.
- Key compaction is disabled by default.
- Enabled compaction improves second-wave hit rate.

## M4: Slug and Variant Intelligence

Goal: expand public object evidence to safe slug routes and bounded variants.

- [ ] M4.1 Add slug-like segment classifier.
- [ ] M4.2 Track slug value hashes without storing raw values.
- [ ] M4.3 Add sensitive-resource override for slug routes.
- [ ] M4.4 Add slug public object candidate logic.
- [ ] M4.5 Track configured variant dimensions.
- [ ] M4.6 Block unbounded variant cardinality.
- [ ] M4.7 Keep unsupported `Vary` as hard protection.
- [ ] M4.8 Add integration tests for public slug and sensitive slug routes.

Acceptance:

- `/articles/{slug}` can become public object candidate.
- `/users/{slug}` remains protected by default.
- Variant evidence is bounded and visible.

## M5: Canary Validation, Demotion, and Purge

Goal: continuously validate promoted precision decisions.

- [ ] M5.1 Add deterministic canary sampler.
- [ ] M5.2 Apply canary to promoted routes and compacted query groups.
- [ ] M5.3 Record canary match/mismatch evidence.
- [ ] M5.4 Demote and purge on canary mismatch.
- [ ] M5.5 Add scoped purge for query-equivalence and variant groups.
- [ ] M5.6 Add cooldown events.
- [ ] M5.7 Add integration tests for canary match and mismatch.

Acceptance:

- Canary mismatch prevents future hits.
- Purge scope matches the failed proof.
- Canary output is bounded and testable.

## M6: Observability, CLI, Docs, and Examples

Goal: make precision behavior actionable.

- [ ] M6.1 Extend route snapshots with confidence, blockers, evidence age, and
  cooldown.
- [ ] M6.2 Extend query snapshots with equivalence state.
- [ ] M6.3 Update dashboard route list and detail.
- [ ] M6.4 Update `kubio routes`.
- [ ] M6.5 Update `kubio explain`.
- [ ] M6.6 Add precision debug headers.
- [ ] M6.7 Add precision metrics and events.
- [ ] M6.8 Update README, configuration, how-decides, safety model, metrics,
  examples, roadmap, and release notes.

Acceptance:

- Users can identify the next action for query compaction candidates.
- Metrics and snapshots do not expose raw path or query values.

## M7: Benchmarks and Release Hardening

Goal: prove v0.5.1 improves precision without safety regressions.

- [ ] M7.1 Add query-noisy public object benchmark.
- [ ] M7.2 Add slug public object benchmark.
- [ ] M7.3 Add sensitive slug benchmark.
- [ ] M7.4 Add evidence decay benchmark.
- [ ] M7.5 Add canary mismatch benchmark.
- [ ] M7.6 Compare v0.5.1 precision scenarios against v0.5.0 baseline.
- [ ] M7.7 Run full workspace tests.
- [ ] M7.8 Run HTTP/3 feature tests.
- [ ] M7.9 Run privacy regression tests.
- [ ] M7.10 Bump workspace version to `0.5.1`.
- [ ] M7.11 Add release notes.

Acceptance:

- Query-noisy and slug public routes show materially higher hit rates than
  v0.5.0 when precision features are enabled.
- Protected user and sensitive query tests remain zero-hit and zero-store by
  default.
- Release docs clearly state that key compaction is stricter than route
  promotion.
