# kubio v0.5.3 Design Index

Status: implemented
Source: v0.5.2 response-header equivalence implementation
Target release: `v0.5.3`

v0.5.0 through v0.5.2 made adaptive reuse more useful and more precise. They
also added operator-controlled route, query, and response-header hints. Today,
applying those controls requires restarting the proxy, which resets process
state and can interrupt local development or small production deployments.

v0.5.3 makes the control loop shorter without broadening cache safety:
operators can validate and apply safe config changes while kubio is running,
and kubio keeps serving with the previous good config if a reload fails.

The release theme is:

```text
Reload hints, not sockets: apply safe policy changes live and keep structural
changes restart-bound.
```

## Problem Statement

The adaptive reuse line depends on evidence and operator intent. A user may see
that `utm_source` is a verified query ignore candidate, or that
`x-vendor-execution-id` is a verified response-header candidate, then add a
route hint to the config file.

The current runtime shape makes that workflow clumsy:

- the user edits config;
- the user restarts kubio;
- existing in-memory route evidence is lost;
- in-flight requests may be interrupted;
- a bad config edit can turn a small tuning change into downtime.

v0.5.3 adds runtime config reload for the parts of config that are safe to
swap inside a running process. Listener sockets, TLS identity, store backend,
origin endpoint, and protocol topology stay restart-required in this release.

## Implementation Snapshot

The shipped implementation adds a process-local active runtime generation,
admin API reload/check/status endpoints, CLI `kubio config` commands, Unix
SIGHUP reload, dashboard visibility, bounded metrics, bounded events, and
documentation/examples. Reload attempts are serialized. Successful reloads swap
the active config, policy engine, and route hint lookup together; failed reloads
leave the active generation unchanged.

State reconciliation is conservative:

- changed or removed route hints purge cache entries for those routes and
  demote matching observer state;
- global policy compatibility changes purge the cache and demote all observed
  routes;
- purge or reconciliation failure rejects the reload before publishing the new
  generation.

Items intentionally deferred from v0.5.3 are recorded in
[Testing and Release](testing-release.md) and [Implementation Tasks](tasks.md):
reload duration histogram metrics, a route-heavy diff benchmark, and broader
reload stress/privacy test suites beyond the shipped unit, router, workspace,
HTTP/3, and reload-smoke coverage.

## Design Documents

- [PRD](PRD.md)
  - Product goals, user experience, non-goals, and success metrics.
- [Runtime Config Reload](runtime-config-reload.md)
  - Reloadable surface, restart-required surface, config generation, and
    command/API flow.
- [Reload Safety and State](reload-safety-and-state.md)
  - Atomic swap model, policy/store/observer behavior, evidence retention,
    demotion, and rollback.
- [Observability and Dashboard](observability-dashboard.md)
  - Dashboard/API fields, CLI output, debug headers, events, and metrics.
- [Testing and Release](testing-release.md)
  - Unit, integration, concurrency, privacy, compatibility, and release gates.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone work breakdown with acceptance checks.

## In Scope

- Runtime reload for safe behavioral config:
  - mode;
  - freshness profile;
  - policy thresholds and adaptive reuse settings;
  - query intelligence settings;
  - response-header equivalence settings;
  - route hints;
  - debug headers;
  - panic file path;
  - selected observability display settings that do not require rerouting.
- Config validation before applying any reload.
- Keep the previous active config when parse, validation, compatibility, or
  apply checks fail.
- Admin API endpoint and CLI command for explicit reload.
- SIGHUP reload on Unix platforms.
- Config generation IDs and redacted active config visibility.
- Compatible observer evidence is preserved unless route or global policy
  compatibility changes require demotion.
- Demote or purge affected evidence and cache entries when route or policy
  changes invalidate prior proof.
- Dashboard, CLI, debug-header, metrics, and event explanations for reload
  success, failure, and restart-required changes.

## Out of Scope

- Reloading proxy listen addresses, dashboard listen addresses, TLS
  certificate/key paths, HTTP/2 listener settings, HTTP/3 listener settings,
  Alt-Svc authorities, or metrics route registration.
- Reloading origin URL, origin protocol topology, origin CA files, or origin
  connection pool settings.
- Reloading storage kind, disk path, maximum store size, or sync behavior.
- Distributed config coordination.
- Hot plugin loading.
- Watching config files automatically by default.
- Persisting observer evidence across process restarts.
- Changing the v0.5.2 cache safety model.

## Cross-Cutting Constraints

- Failed reloads must leave the active runtime unchanged.
- A reload must never make a stored response reusable unless the new config and
  existing hard safety gates allow it.
- Route hint changes that broaden reuse must require fresh validation or
  compatible retained evidence before serving hits.
- Route hint changes that narrow reuse must take effect immediately and purge or
  quarantine affected cache entries.
- Restart-required fields must be detected and reported before partial apply.
- In-flight requests may finish using the config generation they started with.
  New requests must use the latest committed generation.
- Admin reload APIs require the existing admin-token protection when configured.
- Reload observability must expose file paths, generations, field classes,
  counts, route templates, and hashes, not secrets or raw header/query values.

## Milestone Status

- [x] M0: Design, terminology, and reload contract.
- [x] M1: Config source, diff, validation, and generation model.
- [x] M2: Runtime config handle and atomic apply.
- [x] M3: Policy, observer, route hint, and cache state reconciliation.
- [x] M4: Admin API, CLI command, SIGHUP, dashboard, and metrics.
- [x] M5: Observability, dashboard, docs, and examples.
- [x] M6: Tests, compatibility checks, benchmarks, and release hardening.
