# Release Notes v0.5.1

v0.5.1 refines v0.5.0 adaptive reuse with precision evidence.

Highlights:

- Confidence tiers for adaptive routes: `unknown`, `probation`, `validated`,
  `strong`, `cooldown`, and `hard_protected`.
- Evidence decay and cooldown so old confidence does not keep a route promoted
  forever.
- Verified query equivalence: noisy parameters can become
  `verified_ignore_candidate` after matching fingerprint evidence.
- Route-enabled query key compaction through `query.verified_ignore`.
- Sensitive query names remain blocked from automatic ignore candidates.
- Conservative slug route intelligence for public routes such as
  `/articles/{slug}`.
- Canary validation for promoted routes and compacted key groups.
- Dashboard, CLI, debug headers, and metrics now expose confidence, canary, and
  query-equivalence state.
- New benchmark scenarios for query-noisy public objects and slug public object
  routes.

Safety notes:

- Authorization, Cookie, unsafe methods, Set-Cookie, private/no-store,
  unsupported `Vary`, sensitive paths, panic switch, and shadow/canary mismatch
  still block reuse.
- Query compaction is stricter than route promotion because it can merge cache
  keys. It is disabled by default unless route config or global config enables
  it after proof.
- Observation metadata stores hashes, bounded classes, and counters, not raw
  path IDs, raw slugs, query values, credentials, cookies, validators, or
  response bodies.
