# Testing and Release Plan

Status: implemented
Target release: `v0.5.3`

This document now records the v0.5.3 verification plan and the shipped
coverage. The release includes the core reload unit/router tests, the full
workspace suite, the HTTP/3 feature suite, clippy, formatting, whitespace
checks, and the reload-smoke benchmark. Broader concurrency/privacy stress
suites and the route-heavy diff benchmark are deferred hardening items.

## Shipped Verification

Completed before handoff:

```text
cargo fmt --all --check
git diff --check
cargo test -p kubio-cli
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --features experimental-http3
cargo run -p kubio-bench -- --scenario reload-smoke --requests 10
```

Reload-specific shipped tests cover:

- config diff classification for route changes, restart-required structural
  fields, and mixed diffs;
- runtime generation swap preserving an old loaded snapshot;
- reload controller success for route-hint reloads;
- reload controller rejection for restart-required changes;
- reload controller `no_config_source`;
- admin-token protection for the reload endpoint;
- active config endpoint generation output;
- reload metric rendering.

## Unit Tests

### Config Source and Precedence

- reload uses the startup config path when one exists;
- reload returns `no_config_source` when the process started without a config
  file;
- startup CLI overrides keep precedence over config file edits;
- redacted config output hides `admin_token`.

### Diff Classification

- route hint changes are reloadable;
- policy threshold changes are reloadable;
- response-header equivalence changes are reloadable;
- `mode`, `freshness`, `debug_headers`, and `panic_file` are reloadable;
- `server.listen`, TLS, protocol listener settings, origin, origin protocol,
  dashboard listener, storage backend, metrics path, and admin token changes are
  restart-required;
- mixed reloadable and restart-required changes reject the full reload.

### Validation

- parse errors do not change active generation;
- invalid values do not change active generation;
- restart-required diffs do not change active generation;
- successful reload increments generation exactly once;
- dry-run returns a diff without changing generation.

### Runtime Handles

- new requests observe the latest generation;
- requests that captured an older generation keep using it consistently;
- active config and policy are swapped together;
- reload attempts are serialized so two concurrent reloads cannot both commit
  from the same base generation.

### State Reconciliation

- unrelated route additions retain existing evidence;
- removed route hints demote affected routes;
- changed query rules demote affected query evidence;
- changed response-header rules demote affected header evidence;
- stricter thresholds demote routes that no longer qualify;
- disabled adaptive reuse prevents new hits;
- purge failure prevents the reload from committing.

## Integration Tests

### Valid Route Hint Reload

1. Start kubio with a config file.
2. Observe a verified query ignore candidate.
3. Edit the file to enable the route hint.
4. Trigger reload through the admin API or CLI.

Expected:

- reload status is `applied`;
- generation increments;
- compatible evidence is retained or reclassified;
- new requests use the route hint;
- existing v0.5.1 query safety rules still apply.

### Response Header Hint Reload

1. Observe a verified `x-vendor-execution-id` candidate.
2. Add route `response_headers.verified_ignore.allow`.
3. Reload.

Expected:

- new generation applies the header ignore only for that route;
- hits do not replay stripped one-shot headers;
- disabling the hint in a second reload demotes and purges affected entries.

Status: deferred as a dedicated reload integration test. Existing
response-header equivalence integration tests remain green, and route/global
reload reconciliation is covered by controller and observer behavior.

### Invalid Config Reload

Edit the file with an invalid threshold or malformed YAML.

Expected:

- reload returns parse or validation failure;
- active generation stays the same;
- previous config continues to serve requests;
- no cache purge occurs.

Status: partially covered by validation and reload-controller failure paths;
dedicated malformed YAML API integration coverage is deferred.

### Restart-Required Reload

Change `server.listen` or `storage.kind`.

Expected:

- reload returns `restart_required`;
- active generation stays the same;
- CLI/API reports the field path;
- no partial route or policy changes are applied.

### In-Flight Request Consistency

Use an origin endpoint that delays the response. Start a request, trigger a
valid reload, then let the request finish.

Expected:

- delayed request uses its starting generation consistently;
- later requests use the new generation;
- no panic or partial response occurs.

Status: runtime snapshot consistency is covered by the `RuntimeHandle` unit
test. A delayed-origin integration test is deferred.

### SIGHUP Reload

On Unix:

- SIGHUP with valid config applies reload;
- SIGHUP with invalid config logs and records failure but keeps serving;
- SIGHUP without a startup config source records `no_config_source`.

Status: the Unix SIGHUP handler is implemented and shares the reload
controller. Direct signal integration coverage is deferred.

## Concurrency Tests

- multiple concurrent reload API calls serialize safely;
- reload during high request volume does not deadlock;
- reload during dashboard snapshot does not deadlock;
- reload during purge is serialized or produces a deterministic conflict;
- reload during disk store activity does not serve entries after a required
  purge failure.

Status: reload attempts are serialized by an async mutex in the reload
controller. Expanded load/deadlock stress coverage is deferred.

## Privacy Gates

Use config and traffic values:

```text
admin_token: raw-admin-token
Authorization: Bearer raw-secret-token
Cookie: session=raw-cookie-secret
token=raw-query-secret
X-Response-Id: raw-response-id
X-Vendor-Execution-Id: raw-vendor-id
```

Assert raw values do not appear in:

- reload API responses;
- config diff output;
- dashboard HTML;
- metrics;
- events;
- debug headers;
- CLI output.

Status: existing redaction and sensitive-value tests remain green; a dedicated
reload privacy fixture containing all listed values is deferred.

## Compatibility Gates

- v0.5.2 config files load without reload config additions;
- if reload is never triggered, serving behavior matches v0.5.2;
- config API remains backward compatible or gets a new versioned endpoint;
- admin purge behavior remains unchanged;
- memory and disk stores both handle reload reconciliation;
- HTTP/3 feature builds use the same reload safety rules.

## Benchmark and Load Gates

Add a reload smoke benchmark:

```text
steady public-object requests
trigger 10 route-hint reloads
verify no failed requests
verify generation changes are visible
verify hit rate recovers after reload
```

Add a route-heavy diff benchmark:

```text
1,000 route hints
reload with 10 changed hints
measure diff and reconciliation latency
```

Status: reload-smoke benchmark shipped. Route-heavy diff benchmark is deferred.
Budgets should be conservative and local-first. The goal is to catch accidental
O(n^2) diff behavior, not to certify distributed-scale config management.

## Release Gates

Before release:

- [x] Run `cargo fmt --all --check`.
- [x] Run `git diff --check`.
- [x] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [x] Run `cargo test --workspace`.
- [x] Run `cargo test --workspace --features experimental-http3`.
- [x] Run v0.5.3 reload smoke benchmark.
- [x] Confirm docs describe reloadable and restart-required fields.
- [x] Confirm invalid/rejected reloads leave active generation unchanged through
  controller tests.
- [x] Confirm release notes mention that structural changes still require
  restart.
- [-] Run v0.5.0, v0.5.1, and v0.5.2 adaptive benchmarks.
- [-] Run route-heavy diff benchmark.
