# PRD: kubio v0.5.3

Document status: implemented
Target release: `v0.5.3`
Core philosophy: **shorten the operator feedback loop without weakening cache
safety**

## 1. Product Summary

kubio v0.5.3 adds runtime config reload for safe behavioral settings. Operators
can apply route hints, query equivalence hints, response-header equivalence
hints, and policy threshold tuning without restarting the proxy.

The user-facing difference is that a running kubio process can move from
observing a safe opportunity to applying an operator-approved hint in one
validated reload. Existing safe traffic keeps flowing, invalid reloads are
rejected, and structural config changes still ask for a restart.

## 1.1 Implementation Outcome

v0.5.3 ships the core product contract:

- `kubio config check`, `kubio config reload`, `kubio config diff`, and
  `kubio config status`;
- `GET /api/config/active`, `GET /api/config/reload-status`,
  `POST /api/config/check`, and `POST /api/config/reload`;
- Unix SIGHUP reload through the same reload controller used by the API;
- active runtime generations starting at generation `1`;
- atomic publication of config, policy engine, and route hint lookup;
- restart-required rejection for listener, dashboard, origin, storage,
  performance, metrics, and admin-token changes;
- redacted active config visibility, dashboard reload status, per-route reload
  metadata, bounded metrics, and bounded events.

The shipped state reconciliation is conservative. Changed or removed route
hints purge that route and demote its observer state. Global policy
compatibility changes purge all cache entries and demote all observed routes.
If required purge/reconciliation fails, the reload is rejected and the active
generation is unchanged.

The product defers reload duration histograms, a dashboard reload button, a
route-heavy diff benchmark, and broader stress/privacy suites to future
hardening. CLI/API reload, SIGHUP, dashboard visibility, bounded telemetry, and
the normal plus HTTP/3 workspace test suites are included in v0.5.3.

## 2. Background

v0.5.0 introduced adaptive reuse for public object routes. v0.5.1 added
precision evidence, query equivalence, slug route evidence, and canary
validation. v0.5.2 added response-header equivalence and hit-time stripping for
volatile response metadata.

Those releases make the dashboard and CLI more actionable. They can tell the
operator:

- a route is safe but needs explicit query-key compaction;
- a vendor response header looks volatile and needs route enablement;
- a sensitive-looking public route needs an explicit safety override;
- a threshold is too conservative for a local deployment.

Restarting kubio to apply those edits is operationally noisy. It also discards
in-memory evidence that was useful for the decision.

## 3. Goals

v0.5.3 delivered:

1. Add an explicit reloadable config contract.
2. Add an explicit restart-required config contract.
3. Apply safe config changes atomically.
4. Preserve the previous active config on reload failure.
5. Preserve compatible observer evidence across reloads.
6. Demote or purge state when new config invalidates old proof, using
   route-scoped purges for changed/removed route hints and global purges for
   broad policy compatibility changes.
7. Add CLI, admin API, and Unix SIGHUP reload entry points.
8. Expose active config generation, last reload status, and restart-required
   reasons in dashboard, CLI, API, metrics, and events.
9. Keep secrets and raw traffic values out of reload observability.

## 4. User Experience

### 4.1 Apply a Verified Query Hint

The operator sees:

```text
GET /notice/{id}  query_candidate=utm_source  action=enable route query ignore
```

They edit the config:

```yaml
routes:
  - match:
      method: GET
      path: /notice/{id}
    query:
      ignore: ["utm_source"]
```

Then run:

```bash
kubio config reload --dashboard http://127.0.0.1:9900
```

Expected behavior:

- kubio parses and validates the same config file used at startup;
- restart-required fields are unchanged;
- a new config generation is committed;
- new requests use the new route hint;
- compatible existing evidence is retained or reclassified;
- the dashboard shows the new generation and reload timestamp.

### 4.2 Reject a Bad Config Edit

The operator edits:

```yaml
policy:
  min_key_repeats: 0
```

Expected behavior:

- reload validation fails;
- active requests and new requests keep using the previous generation;
- the API and dashboard show the failed generation attempt and validation
  reason;
- no cache entries are purged for a config that was not applied.

### 4.3 Detect Restart-Required Changes

The operator edits:

```yaml
server:
  listen: 0.0.0.0:9090
```

Expected behavior:

- reload is rejected as restart-required;
- the active listener remains unchanged;
- CLI output names `server.listen` as restart-required;
- no partial config changes are applied.

### 4.4 Narrow a Route Hint

The operator removes a route hint that previously enabled response-header
ignore for `x-vendor-execution-id`.

Expected behavior:

- the new config generation applies;
- future fingerprints include that header again unless default policy excludes
  it;
- affected route/header evidence is demoted;
- affected cache entries are purged or quarantined before reuse can happen
  under the new policy.

### 4.5 SIGHUP Reload

On Unix, if kubio started with `--config ./kubio.yml`, sending SIGHUP triggers a
reload of that file.

Expected behavior:

- success and failure are logged and recorded as events;
- no stdout output is written from the serving process;
- if kubio started without a config file, SIGHUP reports that reload has no
  config source and keeps running.

## 5. Non-Goals

v0.5.3 will not:

- reload listener sockets, TLS identity, protocol listener topology, or Alt-Svc
  advertisement authorities;
- reload origin URL, origin protocol topology, origin CA files, or origin
  connection pools;
- reload storage backend, disk path, sync mode, or store capacity;
- introduce automatic file watching by default;
- persist observer evidence across process restarts;
- provide distributed config state;
- cache authenticated responses;
- change existing hard safety denies.

## 6. Product Principles

### 6.1 Reload Is an Atomic Commit

Every reload either commits a full new safe generation or changes nothing.

### 6.2 Structural Config Needs Restart

If a field requires rebinding sockets, rebuilding stores, rebuilding connection
pools, or changing public protocol topology, v0.5.3 calls that out instead of
applying part of the edit.

### 6.3 Evidence Can Survive Only When Compatible

Evidence is retained when the route template, key shaping, policy semantics,
and safety boundaries remain compatible. When compatibility is ambiguous,
kubio demotes to origin validation.

### 6.4 New Config Cannot Bless Old Unsafe Data

Broader reuse settings require validation under the active generation before
serving hits. Narrower settings take effect immediately.

## 7. Success Metrics

The release is successful when:

- valid route/query/header hint changes apply without process restart;
- invalid config changes leave the active generation unchanged;
- restart-required field edits are reported clearly;
- in-flight requests complete while new requests use the committed generation;
- compatible evidence survives a safe reload;
- incompatible route or policy changes demote and purge affected entries;
- admin reload endpoints respect configured admin authentication;
- reload observability contains no admin token, authorization value, cookie
  value, raw query value, raw header value, or response body content;
- existing v0.5.2 adaptive reuse and response-header equivalence tests remain
  green.

Shipped verification covered the workspace test suite, the HTTP/3 feature test
suite, clippy across all targets/features, formatting, whitespace checks, and a
reload-smoke benchmark. Proposed route-heavy diff and broader concurrent reload
stress gates remain follow-up hardening items.
