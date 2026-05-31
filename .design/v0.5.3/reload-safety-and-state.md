# Reload Safety and State

Status: planned
Target release: `v0.5.3`

## Goals

Reload safety ensures that a config edit cannot accidentally reuse unsafe
responses, poison observer evidence, or partially update the runtime.

The runtime should behave as if reload is an atomic commit:

1. build candidate effective config;
2. validate syntax and semantic constraints;
3. compare against active config for restart-required changes;
4. reconcile observer and cache state;
5. publish a new active generation;
6. record the result.

If any step fails, the active generation remains unchanged.

## Runtime Handles

Replace direct long-lived use of one `Arc<EffectiveConfig>` in request paths
with a small config handle:

```rust
pub struct RuntimeConfigHandle {
    current: ArcSwap<ActiveConfig>,
}

impl RuntimeConfigHandle {
    pub fn load(&self) -> Arc<ActiveConfig>;
    pub fn compare_and_swap(&self, expected: u64, next: ActiveConfig) -> Result<()>;
}
```

The exact implementation can use `arc-swap` or an equivalent lock-protected
atomic handle. Reads should stay cheap because every request loads config.

`PolicyEngine` should either become generation-aware and reloadable, or the
runtime should publish a paired policy handle:

```rust
pub struct RuntimePolicyHandle {
    current: ArcSwap<PolicyEngine>,
}
```

The active config and active policy must be swapped together so new requests
cannot observe a mismatched pair.

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

Observer evidence can be retained only when the new config is compatible with
the proof that created the evidence.

Retain evidence for changes such as:

- lowering dashboard-only display options;
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

## Route Reconciliation

Route hints should be diffed by normalized method and path template:

```text
GET /notice/{id}
```

Reconciliation outcomes:

```text
unchanged
added
removed
changed_reloadable
changed_demote
changed_purge
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

Stored entries should carry enough policy metadata to decide whether they can
survive a reload:

```rust
pub struct StoredPolicyMetadata {
    pub config_generation: u64,
    pub fingerprint_policy_version: u16,
    pub route_policy_hash: u64,
    pub key_shape_hash: u64,
    pub response_header_policy_hash: u64,
}
```

v0.5.2 already introduced response-header policy metadata. v0.5.3 should extend
or reuse that shape so reload can identify entries affected by route, query, or
header policy changes.

Compatibility results:

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

If reconciliation requires purging and purge fails, the reload should fail by
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
