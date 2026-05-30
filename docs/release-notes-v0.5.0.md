# Release Notes v0.5.0

Status: implemented.

## Highlights

- Added adaptive reuse so safe public-object traffic can produce useful cache
  hits without waiting for the old high route/key thresholds.
- Added origin-public fast path: a safe `Cache-Control: public` response can be
  stored immediately and reused on the next identical fresh request.
- Added route-level public object evidence using bounded path/key cardinality,
  store-safe rate, and shadow matches.
- Kept hard protection for authenticated requests, cookies, unsafe methods,
  `Set-Cookie`, `private`, `no-store`, unsupported `Vary`, sensitive paths, and
  shadow mismatches.
- Added snapshot fields, dashboard details, CLI output, debug headers, and
  metrics for adaptive reuse classes and blockers.

## Behavior

- `/notice/1` and `/notice/2` normalize to route evidence
  `GET /notice/{id}`, but cache keys still use the raw path, so each notice ID
  remains a separate cache entry.
- `/user/1` remains protected by default because `user` is a sensitive resource
  segment, even when the origin sends public cache headers.
- Public object promotion is reversible. A shadow mismatch protects the route
  and purges route entries before future hits can occur.

## Configuration

```yaml
policy:
  adaptive_reuse:
    enabled: true
    key_validation:
      min_observations: 2
      min_shadow_matches: 1
      max_shadow_mismatches: 0
    public_object:
      enabled: true
      min_route_samples: 20
      min_distinct_keys: 3
      min_store_safe_rate: 0.98
      min_shadow_matches: 3
      max_shadow_mismatches: 0
    origin_public_fast_path:
      enabled: true
```

Route hints may mark a route as a public-object route, but hard denies still
win:

```yaml
routes:
  - match:
      method: GET
      path: "/notice/{id}"
    safety:
      public_object: true
```

## Verification

Implemented checks include:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo test --workspace --features experimental-http3
cargo run -p kubio-bench -- --scenario origin-public-fast-path --requests 4 --output json
cargo run -p kubio-bench -- --scenario exact-key-adaptive --requests 4 --output json
cargo run -p kubio-bench -- --scenario public-object-sweep --requests 12 --output json
cargo run -p kubio-bench -- --scenario protected-user-sweep --requests 6 --output json
```
