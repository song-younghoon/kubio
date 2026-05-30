# Testing and Release Plan

Status: implemented local gates passing
Target release: `v0.5.0`

Verification state:

- [x] `cargo fmt --all --check`
- [x] `cargo check --workspace`
- [x] `cargo test --workspace`
- [x] `cargo test --workspace --features experimental-http3`
- [x] `cargo run -p kubio-bench -- --scenario origin-public-fast-path --requests 4 --output json`
- [x] `cargo run -p kubio-bench -- --scenario exact-key-adaptive --requests 4 --output json`
- [x] `cargo run -p kubio-bench -- --scenario public-object-sweep --requests 12 --output json`
- [x] `cargo run -p kubio-bench -- --scenario protected-user-sweep --requests 6 --output json`

## Unit Tests

### Policy

- Hard request denies still protect.
- Hard response denies still protect or prevent storage.
- Sensitive resource paths classify as hard protected by default.
- Non-sensitive ID routes classify as evidence-gated, not hard protected.
- `key_validated` requires matching fingerprints and zero mismatches.
- `public_object` requires route samples, distinct keys, store-safe rate, and
  zero mismatches.
- `origin_public` requires explicit public cache headers and all hard checks.
- Panic switch blocks adaptive hits and stores.

### Path Intelligence

- Numeric, UUID, ULID, and long hex segments remain ID-like.
- Sensitive resource matching is exact segment matching after percent decode.
- `/notice/1` is not sensitive.
- `/user/1` is sensitive.
- Bounded cardinality classes transition from one to low to medium to high.
- Raw dynamic path values are not present in snapshots or events.

### Observer

- Route evidence increments under safe origin responses.
- Store-safe rate is computed from bounded counters.
- Shadow mismatch demotes promoted routes.
- Demotion records events and prevents future hits.
- Key evidence and route evidence are independent.
- Snapshot sorting prioritizes actionable public object candidates.

## Integration Tests

### Public Object Reuse

Origin routes:

```text
/notice/:id
/user/:id
```

Cases:

- `/notice/1` repeated three times should produce a hit by the third request
  under key validation defaults.
- `/notice/{1..N}` first wave should build public object evidence.
- `/notice/{1..N}` second wave should produce hits after public object
  promotion.
- `/user/1` should remain protected and never store by default.

### Origin Public Fast Path

- `Cache-Control: public, max-age=60` stores first safe response and hits on
  second request.
- `Cache-Control: private` never stores.
- `Cache-Control: no-store` never stores.
- `Cache-Control: public` with `Set-Cookie` never stores.
- Public response with unsupported `Vary` never stores.

### Demotion

- A public object route with a later mismatching fingerprint demotes.
- Demotion purges route entries.
- The next request after demotion goes to origin.
- Re-promotion requires fresh evidence.

### Revalidation and Stale

- Adaptive hits still revalidate stale entries with validators.
- Unsafe 304 metadata purges and blocks reuse.
- Stale-if-error remains allowed only by origin or route policy and never while
  panic switch is active.

### Protocol Parity

- HTTP/1.1 and HTTP/2 share adaptive reuse behavior.
- HTTP/3 experimental tests, when enabled, use the same policy and observer
  gates.
- Protocol fallback does not change reuse eligibility.

## Benchmarks

Add scenarios to `kubio-bench` or baseline smoke:

### Exact Key

```text
GET /stable-object/1 x 100
```

Expected:

- v0.5.0 hits after key validation.
- Hit rate is materially higher than v0.4.1 default.

### Public Object Sweep

```text
GET /notice/1..100
GET /notice/1..100
```

Expected:

- first wave builds route evidence and stores eligible entries when allowed;
- second wave produces hits after public object promotion.

### Protected User Sweep

```text
GET /user/1..100
GET /user/1..100
```

Expected:

- zero hits;
- zero stored entries;
- protected reason is `sensitive_resource`.

### Origin Public

```text
GET /public-max-age/1 x 20
```

Expected:

- hit after first safe store;
- TTL bounded by config.

## Safety Regression Gates

Existing gates must remain green:

- Authorization protected.
- Cookie protected.
- unsafe methods protected.
- Set-Cookie not stored.
- private/no-store not stored.
- no-cache requires validator and revalidation.
- Vary wildcard and unsupported Vary not stored.
- shadow mismatch blocks reuse.
- panic switch disables fresh, revalidated, and stale reuse.
- oversized storeable responses are not partially stored.
- large protected responses are streamed and not stored.

## Privacy Gates

Use test values:

```text
/notice/raw-id-should-not-leak
Authorization: Bearer raw-secret-token
Cookie: session=raw-cookie-secret
?token=raw-query-token
ETag: raw-validator
```

Assert snapshots, metrics, events, and debug headers do not contain raw secret
or identifier values.

## Release Gates

Before release:

- Run full workspace tests.
- Run proxy integration tests without HTTP/3.
- Run HTTP/3 integration tests for the experimental feature where CI supports
  them.
- Run local benchmark smoke and compare adaptive scenarios against v0.4.1
  baseline.
- Confirm docs and examples describe the hard-deny limits clearly.
- Confirm `README.md`, `docs/how-kubio-decides.md`, `docs/safety-model.md`,
  `docs/configuration.md`, `docs/metrics.md`, `docs/roadmap.md`, and release
  notes are updated during implementation.
