# v0.2.0 Release Notes Draft

kubio v0.2.0 adds safer real-world reuse while keeping v0.1.0 hard safety denies.

## Highlights

- Conditional revalidation with `ETag` and `Last-Modified`.
- `Cache-Control: no-cache` support as store-with-revalidation when a validator exists.
- Bounded stale-if-error when origin headers or route policy explicitly allow stale recovery.
- Route hints for freshness, query key behavior, stale-if-error, and force-protect behavior.
- Query key hints with ignored parameter patterns such as `utm_*`.
- Optional process-local disk store.
- Dashboard/API/CLI fields for revalidation, stale responses, and store state.
- Prometheus metrics for revalidation outcomes, stale served/denied counts, and store kind.

## Safety Notes

- Authorization, Cookie, unsafe methods, Set-Cookie, private, no-store, unsupported Vary, Vary wildcard, range requests, and shadow mismatches remain protected.
- Panic switch disables fresh, revalidated, and stale reuse.
- Disk storage persists only responses that passed store safety checks.
- Raw sensitive header values and raw query values are not exposed in metrics or dashboard APIs.

## Verification

Required local checks:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```
