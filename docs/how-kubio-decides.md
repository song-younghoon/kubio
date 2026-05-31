# How kubio Decides

kubio uses deterministic rules, not machine learning.

Every request starts with safety checks. kubio protects unsafe methods, authenticated requests, cookie-based requests, range requests, GET/HEAD requests with bodies, and sensitive-looking paths.

Every origin response is checked before storage or reuse. kubio does not store responses with `Set-Cookie`, `Cache-Control: no-store`, `private`, `Vary: *`, unsupported `Vary` headers, non-200 statuses, missing fingerprints, or oversized bodies.

`Cache-Control: no-cache` can be stored only when the response has an `ETag` or `Last-Modified` validator. kubio revalidates it with origin before every reuse.

## Route States

- Watching: kubio is observing only.
- Candidate: repeated safe traffic was observed.
- Shadow validated: repeated responses matched in shadow validation.
- Auto: kubio may reuse fresh verified responses.
- Protected: kubio found a risk signal or mismatch.

## Adaptive Reuse

v0.5.x separates hard protection from evidence-gated reuse.

- `key_validated`: one exact cache key has repeated with matching fingerprints.
- `origin_public`: the origin explicitly sent a safe `Cache-Control: public`
  response, so the first safe response may be stored and the second identical
  fresh request may hit.
- `public_object_candidate`: a route has bounded high-cardinality object
  evidence, such as `/notice/{id}`.
- `public_object`: the route has enough samples, distinct keys, store-safe
  responses, and shadow matches to store safe first responses for new keys.
- `hard_protected`: a non-negotiable safety signal was observed.

Route evidence uses normalized route IDs, but cache entries remain exact-key
entries. `/notice/1` and `/notice/2` can share route confidence while staying
separate cache objects. `/user/1` remains protected by default because `user` is
a sensitive path segment.

## Precision Adaptive Reuse

v0.5.1 adds finer proof:

- `confidence_tier`: routes move through `unknown`, `probation`, `validated`,
  `strong`, and `cooldown` based on fresh bounded evidence.
- `verified_ignore_candidate`: a query parameter has matching fingerprints
  across multiple value hashes and may be explicitly enabled for key compaction.
- `query_compacted`: a route hint enabled verified query ignore, so proven noisy
  parameters are removed from cache-key construction.
- public slug routes such as `/articles/{slug}` can collect object-route
  evidence; sensitive slug routes remain protected.
- canary validation occasionally sends promoted-route traffic to origin to
  refresh confidence and demote on mismatch.

Key compaction is never automatic by default. It requires proof and route-level
enablement or an explicit global auto-compaction setting.

## Response Header Equivalence

v0.5.2 excludes curated per-request response metadata such as `date`,
`x-request-id`, `x-response-id`, `x-correlation-id`, and trace headers from the
response fingerprint. Cache-safety headers, validators, and representation
headers remain fingerprinted.

Ignoring a header for fingerprinting does not mean replaying it from cache is
correct. kubio strips one-shot volatile metadata from cache-hit responses by
default and may add `Age`.

## Shadow Validation

When the same cache key appears again, kubio compares the latest origin response fingerprint with the previous one. Matching fingerprints increase confidence. Any mismatch blocks automatic reuse.

kubio requires recent shadow validations with zero mismatches before adaptive
route promotion. A mismatch protects the route and prevents future hits.

## Revalidation and Stale Recovery

When a verified cache entry becomes stale, kubio uses `If-None-Match` or `If-Modified-Since` to ask the origin whether the response changed. A `304 Not Modified` response refreshes the stored entry. A `200 OK` response replaces it if the response is still safe.

kubio serves stale during origin errors only when `Cache-Control: stale-if-error` or a route hint explicitly allows it, and only within the configured stale window.

## Runtime Reload

v0.5.3 publishes config generations atomically. Each request captures one
generation at request start and uses that generation's config, policy engine,
and route hints through completion. New requests use the latest applied
generation.

Reloads can change mode, freshness, policy, route hints, debug headers, and the
panic-file path. Structural changes such as listeners, origin, storage,
dashboard binding, metrics path, performance limits, and admin token are
reported as restart-required and do not partially apply.

When route hints or global policy compatibility changes, kubio purges affected
cache entries and demotes affected route/key evidence before publishing the new
generation. If that reconciliation fails, the old generation remains active.
