# Reload Safety and State

Status: implemented
Target release: `v0.5.3`

## Goals

Reload safety ensures that a config edit cannot accidentally reuse unsafe
responses, poison observer evidence, or partially update the runtime.

The runtime behaves as an atomic commit for reloadable runtime state:

1. build candidate effective config;
2. validate syntax and semantic constraints;
3. compare against active config for restart-required changes;
4. reconcile observer and cache state;
5. publish a new active generation;
6. update observer policy thresholds for snapshots/eligibility;
7. record the result.

If any step fails, the active generation remains unchanged.

## Runtime Handles

Direct long-lived use of one `Arc<EffectiveConfig>` in request paths was
replaced by `RuntimeHandle`:

```rust
pub struct RuntimeHandle {
    current: Arc<RwLock<Arc<ActiveRuntime>>>,
}

pub struct ActiveRuntime {
    pub generation: u64,
    pub loaded_at_unix_ms: u64,
    pub config: Arc<EffectiveConfig>,
    pub policy: Arc<PolicyEngine>,
    pub(crate) route_hints: Arc<RouteHintLookup>,
}

impl RuntimeHandle {
    pub fn load(&self) -> Arc<ActiveRuntime>;
    pub fn generation(&self) -> u64;
    pub fn active_config(&self) -> Arc<EffectiveConfig>;
    pub fn replace_config(&self, config: Arc<EffectiveConfig>) -> Result<Arc<ActiveRuntime>>;
}
```

The lock-protected snapshot keeps reads cheap and publishes active config,
policy, and route hints together so new requests cannot observe a mismatched
pair.

## Request Consistency

At request start:

```rust
let runtime = state.runtime.load();
```

The request uses `runtime.config`, `runtime.policy`, and `runtime.generation`
through completion. A request that started on generation `3` may complete after
generation `4` is active. That is acceptable because new requests use
generation `4`, and the old request was evaluated consistently.

Debug headers may include:

```text
x-kubio-config-generation: 4
```

when debug headers are enabled.

## Evidence Retention

Observer evidence is retained only when the new config is compatible with the
proof that created the evidence. The shipped implementation uses conservative
route/global demotion instead of fine-grained proof hashing.

Retain evidence for changes such as:

- enabling debug headers;
- adding an unrelated route hint;
- changing thresholds in a stricter direction when current evidence still
  satisfies the new thresholds;
- adding response-header ignore enablement for a header that is already a
  verified candidate.

Demote evidence for changes such as:

- removing or changing a route hint for that route;
- changing query include or ignore rules for that route;
- changing response-header `verified_ignore`, `force_include`, or
  `preserve_on_hit` for that route;
- changing precision reuse thresholds so existing evidence no longer qualifies;
- changing sensitive path rules or route safety overrides;
- changing `max_object_size` below sizes used by stored entries;
- disabling adaptive reuse, query intelligence, or response-header
  equivalence.

In v0.5.3, any global policy compatibility change listed by
`ConfigDiff::requires_global_cache_purge()` demotes all observed routes and
purges the cache. Changed or removed route hints demote matching routes and
purge matching route cache entries. Added route hints are applied without
resetting unrelated observed routes.

## Route Reconciliation

Route hints are diffed by normalized method and path template:

```text
GET /notice/{id}
```

Reconciliation outcomes:

```text
unchanged
added
removed
changed
demoted
purged
retained
requires_revalidation
```

Rules:

- added route hints apply to new observations immediately;
- removed route hints demote affected routes and purge entries whose reuse
  depended on the removed hint;
- changed hints that broaden reuse require retained compatible evidence or new
  validation before hits;
- changed hints that narrow reuse take effect immediately and purge affected
  entries if the old hint could have influenced stored entries.

## Cache Entry Compatibility

The original design considered adding per-entry policy compatibility metadata:

```rust
pub struct StoredPolicyMetadata {
    pub config_generation: u64,
    pub fingerprint_policy_version: u16,
    pub route_policy_hash: u64,
    pub key_shape_hash: u64,
    pub response_header_policy_hash: u64,
}
```

v0.5.3 did not add this stored metadata. Instead it enforces safety by purging
before commit whenever compatibility is not definitely unchanged:

- changed or removed route hints purge entries for those routes;
- global policy compatibility changes purge all entries;
- purge failure returns `state_reconciliation_failed` and the old generation
  remains active.

This is more conservative than per-entry quarantine, but it keeps reload safety
simple and avoids serving entries whose old proof no longer matches the new
config.

The deferred compatibility states remain future design options:

```text
compatible
requires_revalidation
purge
legacy_unknown
```

Default behavior:

- compatible entries may remain available;
- `requires_revalidation` entries are passed through or revalidated before hit;
- `purge` entries are evicted before new generation reuse;
- `legacy_unknown` entries are treated conservatively.

## Broadening vs Narrowing Policy

Broadening examples:

- `mode: shadow` to `auto`;
- lower `min_key_repeats`;
- enable route query ignore;
- enable verified response-header ignore;
- add public object safety override.

Broadening rules:

- never serve a cache hit solely because the new config says it is allowed;
- require compatible retained evidence or new generation evidence;
- canary validation continues to apply where configured.

Narrowing examples:

- `mode: auto` to `watch`;
- disable adaptive reuse;
- remove query ignore;
- force-include a response header;
- increase required evidence thresholds.

Narrowing rules:

- take effect for new requests immediately;
- demote affected route/key/header state;
- purge or quarantine entries that no longer satisfy policy.

## Failure and Rollback

Potential failure points:

- file read error;
- parse error;
- validation error;
- restart-required diff;
- policy construction error;
- observer reconciliation error;
- store purge error.

If reconciliation requires purging and purge fails, the reload fails by
default. Keeping the previous config is safer than applying a stricter config
while stale entries that depend on the old config remain reusable.

If a best-effort purge mode is later added, it needs a separate design.

## Admin Auth and Secrets

Reload must respect the existing dashboard admin API model:

- if `admin_api` is disabled, reload endpoints return 404;
- if `admin_token` is set, reload endpoints require auth;
- redacted config output must continue to hide `admin_token`;
- config diff must classify secret changes without exposing values.

Changing `admin_token` at runtime is restart-required in v0.5.3.
