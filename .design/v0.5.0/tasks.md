# v0.5.0 Implementation Tasks

Status: implemented
Target release: `v0.5.0`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.5.0 scope

## Current Implementation Snapshot

v0.4.1 baseline exists:

- Watch, shadow, and auto modes.
- Conservative hard-deny policy.
- Shadow validation and auto reuse after route/key confidence.
- Route hints and query key hints.
- Query intelligence with bounded value/fingerprint samples.
- Revalidation, stale-if-error, in-memory and disk stores.
- HTTP/1.1, HTTP/2, and experimental HTTP/3 support.
- Dashboard, JSON APIs, metrics, debug headers, admin purge, and docs.
- Multi-platform install/update release assets.

The v0.5.0 gap is effective reuse. Safe public object routes do not hit often
enough under default thresholds.

## M0: Design and Terminology

Goal: lock adaptive reuse semantics before changing runtime behavior.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.5.0 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add adaptive reuse policy design.
- [x] M0.1.4 Add path intelligence design.
- [x] M0.1.5 Add observability/dashboard design.
- [x] M0.1.6 Add testing and task breakdown.

Acceptance:

- `/notice/{id}` and `/user/{id}` behavior is explicitly documented.
- Hard denies are listed and remain non-negotiable by default.
- Route-level evidence and key-level evidence are separate concepts.

## M1: Policy Taxonomy and Config

Goal: split hard protection from evidence-gated reuse.

- [x] M1.1 Add adaptive reuse config types with defaults.
- [x] M1.2 Add `ReuseClass`, `ReuseSource`, and `AdaptiveBlocker` enums.
- [x] M1.3 Refactor request policy to classify hard denies separately from
  evidence-gated signals.
- [x] M1.4 Refactor response policy to expose store-safe and hard-denied
  outcomes separately.
- [x] M1.5 Preserve legacy config compatibility.
- [x] M1.6 Add unit tests for hard deny taxonomy.
- [x] M1.7 Add unit tests for origin-public fast-path eligibility.

Acceptance:

- Authorization, Cookie, unsafe methods, Set-Cookie, private, no-store,
  unsupported Vary, Vary wildcard, status not cacheable, missing fingerprint,
  panic switch, and shadow mismatch still block reuse.
- Non-sensitive ID routes are not hard protected solely because they contain an
  ID.

## M2: Path Intelligence

Goal: collect bounded path evidence for public object classification.

- [x] M2.1 Add path evidence structs to `kubio-observe`.
- [x] M2.2 Track dynamic segment count and bounded distinct key counts per
  route.
- [x] M2.3 Add sensitive resource classification as a first-class route field.
- [x] M2.4 Add cardinality classes for path evidence.
- [x] M2.5 Ensure snapshots expose classes and counts, not raw path values.
- [x] M2.6 Add tests for `/notice/1` and `/user/1`.
- [x] M2.7 Add privacy tests for raw ID non-leakage.

Acceptance:

- `/notice/{id}` can become `public_object_candidate`.
- `/user/{id}` remains `hard_protected` by default.
- No raw dynamic path segment values appear in snapshots, metrics, or events.

## M3: Adaptive Store and Hit Flow

Goal: make adaptive eligibility affect real cache behavior.

- [x] M3.1 Add key-level evidence tracking and `key_validated` eligibility.
- [x] M3.2 Add public object candidate and promotion logic.
- [x] M3.3 Allow exact-key hits from `key_validated` in auto mode.
- [x] M3.4 Allow route-level public object evidence to store safe first
  responses for new keys.
- [x] M3.5 Allow repeated keys under public object routes to hit on fresh
  stored entries.
- [x] M3.6 Add origin-public first-store and second-hit behavior.
- [x] M3.7 Keep `watch` and `shadow` non-serving while collecting evidence.
- [x] M3.8 Add integration tests for exact-key, public object, and
  origin-public flows.

Acceptance:

- Repeated `/notice/1` can hit by the third request under default v0.5.0
  thresholds.
- `/notice/{1..N}` second wave can hit after public object promotion.
- Cache keys remain raw-path specific.

## M4: Demotion and Purge

Goal: make adaptive promotion reversible and safe.

- [x] M4.1 Demote public object routes on shadow mismatch.
- [x] M4.2 Purge route entries on mismatch or unsafe metadata after promotion.
- [x] M4.3 Record bounded demotion and purge events.
- [x] M4.4 Prevent adaptive hits while panic switch is active.
- [x] M4.5 Add tests for demotion, purge, and re-promotion.

Acceptance:

- A mismatching public object response prevents future hits.
- Purged entries are not served after demotion.
- Demotion does not expose raw path IDs.

## M5: Observability, CLI, and Docs

Goal: make adaptive behavior inspectable.

- [x] M5.1 Add reuse class/source/blocker fields to snapshots.
- [x] M5.2 Add dashboard route list and route detail fields.
- [x] M5.3 Add adaptive events.
- [x] M5.4 Add adaptive metrics with bounded labels.
- [x] M5.5 Add debug headers for reuse source and blocker.
- [x] M5.6 Update `kubio routes` output.
- [x] M5.7 Update `kubio explain` output.
- [x] M5.8 Update README, configuration, safety model, how-decides, metrics,
  roadmap, examples, and release notes.

Acceptance:

- Users can tell whether a route is hard protected or waiting for evidence.
- Metrics and snapshots do not contain raw path IDs or sensitive values.

## M6: Benchmarks and Release Hardening

Goal: prove v0.5.0 improves hit rate without safety regressions.

- [x] M6.1 Add exact-key adaptive benchmark.
- [x] M6.2 Add public object sweep benchmark.
- [x] M6.3 Add protected user sweep benchmark.
- [x] M6.4 Add origin-public fast-path benchmark.
- [x] M6.5 Compare adaptive scenarios against v0.4.1 baseline.
- [x] M6.6 Run full workspace tests.
- [x] M6.7 Run proxy integration tests.
- [x] M6.8 Run HTTP/3 experimental tests where available.
- [x] M6.9 Bump workspace version to `0.5.0`.
- [x] M6.10 Add release notes.

Acceptance:

- Public object and exact-key benchmarks show materially higher hit rates than
  v0.4.1 defaults.
- Existing safety regression tests remain green.
- Release docs state the hard-deny limits clearly.

## Deferred Candidates

- Cookie variance proof for public endpoints that receive irrelevant cookies.
- Operator-configurable sensitive/public resource dictionaries.
- Slug path cardinality.
- Distributed route evidence.
- GraphQL query reuse.
