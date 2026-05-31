# v0.5.3 Implementation Tasks

Status: planned
Target release: `v0.5.3`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.5.3 scope

## Current Implementation Snapshot

v0.5.2 baseline exists:

- adaptive reuse, precision evidence, query equivalence, public slug evidence,
  canary validation, and response-header equivalence;
- route hints for freshness, query shaping, response headers, vary,
  stale-if-error, and safety;
- admin dashboard API for read-only config and protected purge;
- process-local observer evidence and memory/disk stores;
- serving process builds config, policy, observer, and store at startup.

The v0.5.3 gap is runtime reload for safe behavioral config. Structural runtime
changes remain restart-required.

## M0: Design and Reload Contract

Goal: lock reload semantics before runtime changes.

### M0.1 Design Documents

- [x] M0.1.1 Add v0.5.3 design index.
- [x] M0.1.2 Add PRD.
- [x] M0.1.3 Add runtime config reload design.
- [x] M0.1.4 Add reload safety and state design.
- [x] M0.1.5 Add observability/dashboard design.
- [x] M0.1.6 Add testing and task breakdown.

Acceptance:

- Reloadable and restart-required fields are documented.
- Failed reload behavior is documented.
- Evidence retention, demotion, and purge behavior are documented.

## M1: Config Source, Diff, and Validation

Goal: build candidate configs and classify changes safely.

- [ ] M1.1 Store startup config source and CLI overrides.
- [ ] M1.2 Add reusable config load path for startup and reload.
- [ ] M1.3 Add local `kubio config check --config`.
- [ ] M1.4 Add structural config diff with field paths.
- [ ] M1.5 Classify reloadable vs restart-required fields.
- [ ] M1.6 Redact secret values in diff and API output.
- [ ] M1.7 Add validation tests for reloadable fields.
- [ ] M1.8 Add validation tests for restart-required fields.

Acceptance:

- A valid route-hint edit is classified reloadable.
- A listener, origin, storage, or admin-token edit is classified
  restart-required.
- Mixed diffs reject the full reload.

## M2: Runtime Config and Policy Handle

Goal: let request paths load a consistent active generation cheaply.

- [ ] M2.1 Add `ActiveRuntime` or equivalent generation wrapper.
- [ ] M2.2 Add atomic config/policy handle.
- [ ] M2.3 Update proxy state to hold runtime handle instead of static config
  where reloadable behavior is needed.
- [ ] M2.4 Ensure each request captures one generation at request start.
- [ ] M2.5 Rebuild policy engine for candidate configs before commit.
- [ ] M2.6 Serialize reload attempts.
- [ ] M2.7 Add generation to redacted config API or a new active-config API.
- [ ] M2.8 Add tests for request consistency across reload.

Acceptance:

- New requests use the latest committed generation.
- In-flight requests complete with their starting generation.
- Config and policy cannot be observed as mismatched.

## M3: Observer and Cache Reconciliation

Goal: preserve compatible evidence and remove stale proof when config changes.

- [ ] M3.1 Add route hint diff by normalized method/path.
- [ ] M3.2 Add policy hash or compatibility metadata for route/key/header
  evidence.
- [ ] M3.3 Retain evidence for compatible unrelated changes.
- [ ] M3.4 Demote route evidence for changed query, header, safety, or
  threshold semantics.
- [ ] M3.5 Extend stored policy metadata where needed.
- [ ] M3.6 Purge or quarantine cache entries affected by narrowing changes.
- [ ] M3.7 Fail reload if required purge fails.
- [ ] M3.8 Record reconciliation summary for observability.

Acceptance:

- Removing a route hint prevents future hits that depended on it.
- Adding an unrelated route hint does not reset all evidence.
- Purge failure leaves active generation unchanged.

## M4: Admin API, CLI, and SIGHUP

Goal: expose explicit and scriptable reload controls.

- [ ] M4.1 Add protected `POST /api/config/reload`.
- [ ] M4.2 Add `POST /api/config/check` dry-run support.
- [ ] M4.3 Add `GET /api/config/reload-status`.
- [ ] M4.4 Add `kubio config reload`.
- [ ] M4.5 Add `kubio config diff`.
- [ ] M4.6 Add `kubio config status`.
- [ ] M4.7 Add Unix SIGHUP reload when a config source exists.
- [ ] M4.8 Keep reload logs off stdout in serve mode.
- [ ] M4.9 Add admin auth tests.

Acceptance:

- CLI can trigger reload against the default local dashboard URL.
- Unauthorized reload requests fail.
- SIGHUP invalid config leaves the process serving the old generation.

## M5: Observability, Dashboard, Docs, and Examples

Goal: make reload state explainable without exposing secrets.

- [ ] M5.1 Add reload snapshot fields.
- [ ] M5.2 Add reload events.
- [ ] M5.3 Add reload metrics.
- [ ] M5.4 Add config generation debug header.
- [ ] M5.5 Update dashboard config page.
- [ ] M5.6 Update route detail with reload demotion reason.
- [ ] M5.7 Update README and configuration docs.
- [ ] M5.8 Update how-decides, safety model, and metrics docs.
- [ ] M5.9 Add example reload workflow.

Acceptance:

- Operators can see active generation and last reload result.
- Metrics labels are bounded.
- No secret or raw traffic values appear in reload observability.

## M6: Tests, Benchmarks, and Release Hardening

Goal: prove reload is safe under normal and failure conditions.

- [ ] M6.1 Add valid route-hint reload integration test.
- [ ] M6.2 Add response-header hint reload integration test.
- [ ] M6.3 Add invalid config reload integration test.
- [ ] M6.4 Add restart-required reload integration test.
- [ ] M6.5 Add in-flight request consistency test.
- [ ] M6.6 Add SIGHUP tests on Unix.
- [ ] M6.7 Add concurrency tests for reload vs requests, snapshots, and purge.
- [ ] M6.8 Add reload privacy regression tests.
- [ ] M6.9 Add reload smoke benchmark.
- [ ] M6.10 Run full workspace tests.
- [ ] M6.11 Run HTTP/3 feature tests.
- [ ] M6.12 Bump workspace version to `0.5.3`.
- [ ] M6.13 Add release notes.

Acceptance:

- Invalid reloads never change active generation.
- Restart-required changes are reported clearly.
- Safe hint reloads apply without process restart.
- Existing v0.5.2 safety and adaptive reuse behavior remains intact.
