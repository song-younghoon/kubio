# Testing and Release Plan

Status: implemented local gates passing
Target release: `v0.5.2`

## Unit Tests

### Header Taxonomy

- `x-response-id` is excluded from the normalized header hash by default.
- `x-request-id`, `x-correlation-id`, trace headers, cloud request IDs, `date`,
  and `age` are excluded by default.
- `content-type`, `content-encoding`, `cache-control`, `vary`, `etag`,
  `last-modified`, and `expires` remain included by default.
- `set-cookie` still hard-protects storage and reuse.
- sensitive/business-state names such as `x-user-id`, `x-session-id`, and
  `x-feature-flag` cannot become automatic ignore candidates.
- `force_include` overrides default volatile handling.

### Fingerprint Normalization

- two responses with the same status/body/semantic headers but different
  `x-response-id` produce the same v0.5.2 fingerprint.
- changing body hash still changes the fingerprint even when volatile headers
  differ.
- changing representation headers changes the fingerprint.
- changing cache-safety headers changes policy outcome or fingerprint.
- fingerprint policy version is included in stored metadata and comparison.

### Header Candidate Evidence

- an unknown non-sensitive header with changing values and matching
  status/body/semantic headers becomes `verified_volatile_candidate`.
- candidates are not applied automatically when `auto_apply_unknown` is false.
- route enablement applies a verified candidate.
- candidate mismatch moves the route/header group to cooldown.
- raw header values are stored only as bounded hashes where needed.

### Store and Hit-Time Sanitization

- origin misses forward origin one-shot metadata.
- cache hits do not replay suppressed `x-response-id` or trace IDs.
- `Age` is added or updated when configured.
- legacy entries without header policy metadata are handled conservatively.

## Integration Tests

### Public Route With Dynamic Response ID

Origin route:

```text
GET /notice/1
```

Every origin response returns:

```text
X-Response-Id: unique
Content-Type: application/json
```

Expected:

- route/key can pass existing v0.5.1 reuse thresholds;
- second-wave requests hit;
- cache-hit responses omit `x-response-id` by default;
- debug headers explain `x-response-id` normalization when enabled.

### Unknown Candidate Header

Origin returns changing `x-vendor-execution-id`.

Expected:

- kubio reports `verified_volatile_candidate`;
- without route enablement, behavior remains conservative;
- with route enablement, requests can reuse after proof;
- a later semantic mismatch demotes and purges the affected entries.

### Semantic Header Change

Origin returns same body but changes `content-type` or `etag`.

Expected:

- fingerprint mismatch remains visible;
- no automatic ignore candidate is created;
- promoted routes demote if the mismatch invalidates prior proof.

### Safety Header Change

Origin returns `Set-Cookie`, `Cache-Control: private`, `no-store`,
`Vary: *`, or unsupported `Vary`.

Expected:

- storage and reuse are blocked;
- header normalization does not hide the hard deny.

### Protected User Route

`GET /user/1` returns changing `x-response-id`.

Expected:

- sensitive path protection still wins;
- zero stores and zero hits by default.

## Benchmarks

Add `kubio-bench` scenarios:

### Dynamic Response Metadata Public Object

```text
GET /notice/1
GET /notice/1
```

Origin changes:

```text
X-Response-Id
X-Correlation-Id
Date
```

Expected:

- v0.5.1 baseline misses or delays promotion because fingerprints differ;
- v0.5.2 reaches normal exact-key/public-object hit behavior;
- no raw header values appear in output.

### Vendor Header Candidate

```text
GET /feed/1
GET /feed/1
```

Origin changes `x-vendor-execution-id`.

Expected:

- `vendor-header-candidate` reports the candidate opportunity without applying
  it by default;
- `vendor-header-route-enabled` improves hit rate for operator-approved metadata
  names.

### Safety Regression Sweep

Existing protected scenarios plus dynamic metadata headers:

```text
/user/1
/set-cookie
/private
/nostore
/vary-star
/unstable-body
```

Expected:

- protected and unsafe scenarios remain zero-hit.

## Privacy Gates

Use test values:

```text
X-Response-Id: raw-response-id
X-Correlation-Id: raw-correlation-id
Traceparent: raw-traceparent
X-Vendor-Execution-Id: raw-vendor-id
Authorization: Bearer raw-secret-token
Cookie: session=raw-cookie-secret
```

Assert snapshots, metrics, events, debug headers, CLI output, and disk metadata
do not contain raw values.

## Compatibility Gates

- v0.5.1 config files load without adding response-header-equivalence config.
- setting `policy.response_header_equivalence.enabled: false` preserves v0.5.1
  behavior as closely as possible.
- existing disk entries without header policy metadata do not crash the store.
- legacy entries are refreshed or passed through safely when fingerprint policy
  version mismatch prevents a safe comparison.

## Release Gates

Before release:

- Run `cargo fmt --all --check`.
- Run `cargo clippy --all-targets --all-features -- -D warnings`.
- Run `cargo test --workspace`.
- Run `cargo test --workspace --features experimental-http3`.
- Run v0.5.0 and v0.5.1 adaptive benchmarks.
- Run v0.5.2 dynamic-response-header benchmarks.
- Confirm docs describe header normalization, hit-time stripping, and hard
  safety limits.
- Add release notes for v0.5.2.

## Local Verification Results

Date: `2026-05-31`

Passed gates:

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --features experimental-http3`

Benchmark results:

- `dynamic-response-metadata`, 4 requests: 1 origin request, 3 reused
  responses.
- `vendor-header-candidate`, 4 requests: 4 origin requests, 0 reused
  responses, confirming default conservative behavior.
- `vendor-header-route-enabled`, 4 requests: 2 origin requests, 2 reused
  responses.
- Existing adaptive scenarios `exact-key-adaptive`,
  `origin-public-fast-path`, `public-object-sweep`,
  `query-noisy-public-object`, `slug-public-object-sweep`, and
  `sensitive-slug-sweep` passed their local budgets.
