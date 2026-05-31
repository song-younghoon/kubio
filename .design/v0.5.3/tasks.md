# v0.5.3 Implementation Tasks

Status: implemented
Target release: `v0.5.3`

Task states:

- `[ ]` not started
- `[~]` in progress
- `[x]` complete
- `[-]` explicitly deferred from the shipped v0.5.3 scope

## Current Implementation Snapshot

v0.5.3 implementation adds:

- process-local runtime generation with active config, policy engine, and route
  hint lookup published together;
- reusable startup/reload config loading with startup CLI override precedence;
- structural config diff with reloadable and restart-required classes;
- CLI `kubio config check`, `reload`, `diff`, and `status`;
- admin API `GET /api/config/active`, `GET /api/config/reload-status`,
  `POST /api/config/check`, and `POST /api/config/reload`;
- Unix SIGHUP reload through the same reload controller;
- dashboard reload status, active config visibility, route reload metadata,
  bounded metrics, and bounded events;
- conservative state reconciliation through route/global demotion and cache
  purge before commit.

Structural runtime changes remain restart-required. Fine-grained stored-entry
compatibility metadata, reload duration histograms, route-heavy diff
benchmarks, and expanded reload stress/privacy suites are deferred.

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

- [x] M1.1 Store startup config source and CLI overrides.
- [x] M1.2 Add reusable config load path for startup and reload.
- [x] M1.3 Add local `kubio config check --config`.
- [x] M1.4 Add structural config diff with field paths.
- [x] M1.5 Classify reloadable vs restart-required fields.
- [x] M1.6 Redact secret values in diff and API output.
- [x] M1.7 Add validation tests for reloadable fields.
- [x] M1.8 Add validation tests for restart-required fields.

Acceptance:

- A valid route-hint edit is classified reloadable.
- A listener, origin, storage, or admin-token edit is classified
  restart-required.
- Mixed diffs reject the full reload.

## M2: Runtime Config and Policy Handle

Goal: let request paths load a consistent active generation cheaply.

- [x] M2.1 Add `ActiveRuntime` or equivalent generation wrapper.
- [x] M2.2 Add atomic config/policy handle.
- [x] M2.3 Update proxy state to hold runtime handle instead of static config
  where reloadable behavior is needed.
- [x] M2.4 Ensure each request captures one generation at request start.
- [x] M2.5 Rebuild policy engine for candidate configs before commit.
- [x] M2.6 Serialize reload attempts.
- [x] M2.7 Add generation to redacted config API or a new active-config API.
- [x] M2.8 Add tests for request consistency across reload.

Acceptance:

- New requests use the latest committed generation.
- In-flight requests complete with their starting generation.
- Config and policy cannot be observed as mismatched.

## M3: Observer and Cache Reconciliation

Goal: preserve compatible evidence and remove stale proof when config changes.

- [x] M3.1 Add route hint diff by normalized method/path.
- [-] M3.2 Add policy hash or compatibility metadata for route/key/header
  evidence.
- [x] M3.3 Retain evidence for compatible unrelated changes.
- [x] M3.4 Demote route evidence for changed query, header, safety, or
  threshold semantics.
- [-] M3.5 Extend stored policy metadata where needed.
- [x] M3.6 Purge or quarantine cache entries affected by narrowing changes.
- [x] M3.7 Fail reload if required purge fails.
- [x] M3.8 Record reconciliation summary for observability.

Acceptance:

- Removing a route hint prevents future hits that depended on it.
- Adding an unrelated route hint does not reset all evidence.
- Purge failure leaves active generation unchanged.

Deferred notes:

- v0.5.3 ships conservative purge/demotion reconciliation instead of
  fine-grained stored-entry compatibility metadata.

## M4: Admin API, CLI, and SIGHUP

Goal: expose explicit and scriptable reload controls.

- [x] M4.1 Add protected `POST /api/config/reload`.
- [x] M4.2 Add `POST /api/config/check` dry-run support.
- [x] M4.3 Add `GET /api/config/reload-status`.
- [x] M4.4 Add `kubio config reload`.
- [x] M4.5 Add `kubio config diff`.
- [x] M4.6 Add `kubio config status`.
- [x] M4.7 Add Unix SIGHUP reload when a config source exists.
- [x] M4.8 Keep reload logs off stdout in serve mode.
- [x] M4.9 Add admin auth tests.

Acceptance:

- CLI can trigger reload against the default local dashboard URL.
- Unauthorized reload requests fail.
- SIGHUP invalid config leaves the process serving the old generation.

## M5: Observability, Dashboard, Docs, and Examples

Goal: make reload state explainable without exposing secrets.

- [x] M5.1 Add reload snapshot fields.
- [x] M5.2 Add reload events.
- [x] M5.3 Add reload metrics.
- [x] M5.4 Add config generation debug header.
- [x] M5.5 Update dashboard config page.
- [x] M5.6 Update route detail with reload demotion reason.
- [x] M5.7 Update README and configuration docs.
- [x] M5.8 Update how-decides, safety model, and metrics docs.
- [x] M5.9 Add example reload workflow.

Acceptance:

- Operators can see active generation and last reload result.
- Metrics labels are bounded.
- No secret or raw traffic values appear in reload observability.

## M6: Tests, Benchmarks, and Release Hardening

Goal: prove reload is safe under normal and failure conditions.

- [x] M6.1 Add valid route-hint reload controller test.
- [-] M6.2 Add response-header hint reload integration test.
- [x] M6.3 Add invalid/rejected config reload controller tests.
- [x] M6.4 Add restart-required reload controller test.
- [x] M6.5 Add runtime generation consistency test.
- [-] M6.6 Add SIGHUP tests on Unix.
- [-] M6.7 Add concurrency tests for reload vs requests, snapshots, and purge.
- [-] M6.8 Add reload privacy regression tests.
- [x] M6.9 Add reload smoke benchmark.
- [x] M6.10 Run full workspace tests.
- [x] M6.11 Run HTTP/3 feature tests.
- [x] M6.12 Bump workspace version to `0.5.3`.
- [x] M6.13 Add release notes.

Acceptance:

- Invalid reloads never change active generation.
- Restart-required changes are reported clearly.
- Safe hint reloads apply without process restart.
- Existing v0.5.2 safety and adaptive reuse behavior remains intact.

Deferred notes:

- Response-header reload, direct SIGHUP, expanded concurrency, and dedicated
  reload privacy integration tests are follow-up hardening.
- Full workspace and HTTP/3 feature suites passed, preserving existing safety
  and response-header equivalence coverage.
