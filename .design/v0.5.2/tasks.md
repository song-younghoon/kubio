# v0.5.2 Implementation Tasks

Status: proposed
Target release: `v0.5.2`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.5.2 scope

## Current Implementation Snapshot

v0.5.1 baseline exists:

- precision confidence tiers;
- evidence decay and cooldown;
- query equivalence and route-enabled key compaction;
- public slug route evidence;
- canary validation;
- dashboard, CLI, debug headers, metrics, benchmarks, and release notes.

The v0.5.2 gap is response-header equivalence. The current header fingerprint
already ignores a small volatile list, including `date` and `x-request-id`, but
it does not cover `x-response-id` and does not formalize hit-time stripping of
one-shot metadata headers.

## M0: Design and Terminology

Goal: lock v0.5.2 header equivalence semantics before runtime changes.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.5.2 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add response header equivalence design.
- [x] M0.1.4 Add header sanitization and store contract.
- [x] M0.1.5 Add observability/dashboard design.
- [x] M0.1.6 Add testing and task breakdown.

Acceptance:

- Default volatile metadata headers are documented.
- Hard safety and representation headers are documented as
  fingerprint-sensitive.
- Fingerprint ignoring and cache-hit header replay are explicitly separated.

## M1: Header Taxonomy and Config

Goal: add first-class config and types for response-header equivalence.

- [ ] M1.1 Add `ResponseHeaderEquivalenceConfig`.
- [ ] M1.2 Add `ResponseHeaderServeConfig`.
- [ ] M1.3 Add route hint fields for `response_headers.verified_ignore`,
  `force_include`, and `preserve_on_hit`.
- [ ] M1.4 Add default volatile metadata header list.
- [ ] M1.5 Add sensitive/business-state header block patterns.
- [ ] M1.6 Add config validation for thresholds, patterns, and conflicts.
- [ ] M1.7 Add config docs and example config updates.

Acceptance:

- v0.5.1 config files continue to load.
- `force_include` can override default volatile behavior.
- hard safety headers cannot be configured into safe ignore behavior.

## M2: Fingerprint Normalization

Goal: replace implicit stable header hashing with policy-aware normalization.

- [ ] M2.1 Add `HeaderFingerprintPolicy`.
- [ ] M2.2 Add `HeaderFingerprintResult`.
- [ ] M2.3 Include fingerprint policy version in `ResponseFingerprint`.
- [ ] M2.4 Exclude default volatile metadata headers from normalized hashes.
- [ ] M2.5 Keep cache-safety, validator, and representation headers included.
- [ ] M2.6 Update canary/shadow comparison to use normalized fingerprints.
- [ ] M2.7 Add unit tests for default volatile, force-include, and semantic
  header changes.

Acceptance:

- changing only `x-response-id` no longer changes v0.5.2 fingerprints.
- changing body, status, cache-safety headers, or representation headers still
  changes fingerprint or policy outcome.

## M3: Header Equivalence Evidence

Goal: detect unknown volatile header candidates without applying them
automatically by default.

- [ ] M3.1 Add bounded response-header evidence structs.
- [ ] M3.2 Track value hashes by route/header name.
- [ ] M3.3 Track matching fingerprint evidence with each candidate excluded.
- [ ] M3.4 Add `HeaderEquivalenceClass`.
- [ ] M3.5 Add verified candidate promotion.
- [ ] M3.6 Add route-enabled verified ignore application.
- [ ] M3.7 Add mismatch cooldown and scoped purge.
- [ ] M3.8 Add tests for candidate, verified candidate, enabled ignore, and
  demotion.

Acceptance:

- unknown non-sensitive metadata-like headers can become suggestions.
- unknown candidates do not affect fingerprints unless enabled.
- candidate mismatch demotes the affected route/header group.

## M4: Store and Hit-Time Header Sanitization

Goal: avoid replaying one-shot metadata headers from cache hits.

- [ ] M4.1 Add stored header policy metadata.
- [ ] M4.2 Strip default volatile one-shot IDs from stored hit headers.
- [ ] M4.3 Strip verified ignored headers on hits when configured.
- [ ] M4.4 Preserve origin miss headers unchanged except existing proxy-managed
  headers.
- [ ] M4.5 Add or update `Age` on cache hits when configured.
- [ ] M4.6 Handle legacy disk entries without header policy metadata.
- [ ] M4.7 Add integration tests proving hits do not replay `x-response-id`.

Acceptance:

- origin miss responses still include origin request/response IDs.
- cache hits omit suppressed one-shot IDs by default.
- legacy entries are served, refreshed, or passed through safely.

## M5: Observability, CLI, Docs, and Examples

Goal: make header normalization explainable and private.

- [ ] M5.1 Extend route snapshots with header-equivalence counts and blockers.
- [ ] M5.2 Add header candidate snapshots.
- [ ] M5.3 Update dashboard route list and route detail.
- [ ] M5.4 Update `kubio routes`.
- [ ] M5.5 Update `kubio explain`.
- [ ] M5.6 Add debug headers for header shape, ignored names, and suppressed
  names.
- [ ] M5.7 Add metrics and events.
- [ ] M5.8 Update README, configuration, how-decides, safety model, metrics,
  examples, roadmap, and release notes.

Acceptance:

- operators can identify whether a route is blocked by a dynamic response
  header.
- output includes header names/classes/counts, not values.

## M6: Benchmarks and Release Hardening

Goal: prove v0.5.2 improves dynamic-header public routes without regressions.

- [ ] M6.1 Add dynamic response metadata public object benchmark.
- [ ] M6.2 Add vendor header candidate benchmark.
- [ ] M6.3 Add safety regression sweep with dynamic metadata headers.
- [ ] M6.4 Add privacy regression tests for raw header values.
- [ ] M6.5 Run full workspace tests.
- [ ] M6.6 Run HTTP/3 feature tests.
- [ ] M6.7 Compare v0.5.2 dynamic-header hit rates against v0.5.1 baseline.
- [ ] M6.8 Bump workspace version to `0.5.2`.
- [ ] M6.9 Add release notes.

Acceptance:

- dynamic `x-response-id` public routes hit at the same level as equivalent
  stable-header routes.
- protected user and unsafe response scenarios remain zero-hit and zero-store.
- cache-hit responses do not replay stripped one-shot identifiers.
