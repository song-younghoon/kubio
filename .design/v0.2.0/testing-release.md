# Testing and Release

Status: implementation gates passing; release smoke automation added
Target release: `v0.2.0`

## Goals

Testing must prove v0.2.0 did not weaken v0.1.0 safety. The release should be blocked by any regression that serves protected, unvalidated stale, or incorrectly keyed responses.

## Required Existing Gates

Keep v0.1.0 gates:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check
cargo audit
```

If supply-chain tools are unavailable locally, CI must still run them before release.

Current local verification for the implementation and hardening commits:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
git diff --check
```

`cargo deny check` and `cargo audit` remain external release-gate follow-ups when those tools are available locally or in CI.

## Unit Tests

Required new unit areas:

- [x] `Cache-Control` parsing for `max-age`, `no-cache`, `must-revalidate`, and `stale-if-error`.
- [x] Validator extraction and bounds.
- [~] Freshness calculation.
- [~] 304 header merge policy.
- [~] Stale-if-error permission calculation.
- [x] Route hint parsing and conflict validation.
- [~] Route hint specificity sorting.
- [x] Query include/ignore normalization.
- [x] Sensitive query parameter detection.
- [x] Disk store metadata encoding/decoding.
- [x] Store format version rejection.
- [x] New decision reason user messages.

## Integration Tests

### Revalidation

- [x] ETag stale entry sends `If-None-Match`.
- [x] Last-Modified stale entry sends `If-Modified-Since`.
- [x] 304 returns stored body with refreshed metadata.
- [x] 200 revalidation replaces stored body.
- [x] Unsafe 304 metadata causes purge plus safe refetch.
- [x] Missing validator causes origin pass-through.
- [x] `Cache-Control: no-cache` is stored only with validator and revalidated before every use.

### Stale-If-Error

- [x] Origin `stale-if-error` permits stale on origin error within stale window.
- [~] Route hint permits stale on timeout within stale window.
- [~] Stale is denied when no permission exists.
- [~] Stale is denied after `max_stale`.
- [x] Stale is denied when panic switch is active.
- [x] Stale is denied for Authorization, Cookie, Set-Cookie, private, no-store, unsupported Vary, and shadow mismatch cases.
- [x] Revalidation 5xx can serve stale only when allowed.

### Route Hints

- [x] Matching hint applies TTL.
- [x] Matching hint ignores configured query parameter.
- [x] Non-matching route does not use hint.
- [~] Sensitive path acknowledgment only affects sensitive path reason and cannot override personalized signals.
- [x] Force-protect hint protects otherwise safe public routes.
- [x] Conflicting hints fail config validation.

### Query Intelligence

- [x] Query parameter stats are recorded without raw values.
- [x] Repeated query parameter ordering stays deterministic.
- [x] Ignored parameters merge cache keys only on configured routes.
- [x] Dashboard/API suggestions are generated only after sufficient bounded fingerprint evidence.
- [x] Sensitive query parameter names never produce auto-ignore suggestions.

### Disk Store

- [x] Safe entry survives restart.
- [x] Expired entry is not served as fresh after restart.
- [~] Stale entry after restart requires revalidation.
- [x] Protected response is not persisted.
- [x] Purge all/route/key works on disk.
- [x] Single corrupt entry is skipped without crashing hot path.
- [x] Corrupt metadata cannot cause arbitrary file reads.
- [~] Disk write failure returns origin response.

### Observability

- [x] Metrics include revalidation outcomes.
- [x] Metrics include stale served/denied counts.
- [x] Metrics include store kind.
- [x] Dashboard overview includes v0.2.0 fields.
- [~] CLI explain includes revalidation/stale/query details.
- [x] Sensitive values do not appear in logs, metrics, dashboard APIs, or disk metadata.

## Property Tests

Use `proptest` for:

- Query hint normalization never panics.
- Ignored parameters are removed only when matching configured patterns.
- Include/ignore logic is deterministic across arbitrary parameter order.
- Route hint matching is deterministic.
- Header merge never preserves hop-by-hop headers.
- Validator bounds reject overlong values.
- Disk metadata decode rejects malformed input without panic.

## Performance Tests

Initial targets:

- Fresh memory hit remains near v0.1.0 hit overhead.
- Fresh disk hit p95 overhead <= 5ms in local 100 RPS test for 1KiB bodies.
- 304 revalidation p95 overhead is dominated by origin RTT plus <= 3ms local overhead.
- Disk startup recovers 10,000 entries in a documented bounded time.
- Dashboard query stats do not materially affect proxy p95.

Scenarios:

- 100 RPS fresh hits, memory.
- 100 RPS fresh hits, disk.
- 100 RPS stale entries with 304 revalidation.
- Mixed 80% fresh, 10% revalidate, 5% stale-if-error, 5% protected.
- Restart with populated disk cache.

## Security and Privacy Tests

Required assertions:

- Authorization/Cookie/Set-Cookie values do not appear in disk files.
- Raw query values do not appear in metrics or dashboard APIs.
- Validator values are not shown in default dashboard/API output.
- Route hints cannot override hard denies.
- Disk path traversal is impossible through cache key material and disk metadata body file names.
- Corrupt disk entries cannot cause arbitrary file reads or writes.
- Panic switch disables fresh, revalidated, and stale reuse.

## Release Artifacts

v0.2.0 should publish:

- Source tag.
- Linux x86_64 binary.
- Docker image.
- Checksums.
- Release notes.

Optional:

- macOS arm64 binary.
- Example config for disk store and route hints.

## Smoke Test

Release smoke test:

```bash
kubio --help
kubio serve --config examples/kubio-v0.2.yml
curl http://localhost:8080/api/products
curl http://localhost:8080/api/products
curl http://127.0.0.1:9900/api/overview
curl http://127.0.0.1:9900/api/store
curl http://127.0.0.1:9900/metrics
```

Expected:

- Proxy starts.
- Dashboard starts.
- Metrics expose v0.2.0 counters.
- Disk store opens when configured.
- No protected traffic is reused.

Implemented smoke scripts:

```bash
MODE=auto STORAGE_KIND=disk bash examples/bench/local_smoke.sh
KUBIO_BIN=dist/kubio-x86_64-unknown-linux-gnu bash examples/bench/release_smoke.sh
KUBIO_IMAGE=kubio:v0.2.0 bash examples/bench/docker_smoke.sh
```

## Release Exit Criteria

All must be true:

- Existing v0.1.0 safety tests pass.
- Revalidation with ETag and Last-Modified is tested.
- `no-cache` is never reused without revalidation.
- Stale-if-error is explicit, bounded, and tested.
- Route hints cannot override hard denies.
- Query hints are deterministic and redacted.
- Disk store survives restart and handles corrupt entries safely.
- Documentation explains defaults, limits, and migration from v0.1.0.
- Release artifact smoke, Docker smoke, `cargo deny`, and `cargo audit` pass before tagging.
