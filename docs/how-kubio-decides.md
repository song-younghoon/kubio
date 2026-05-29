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

## Shadow Validation

When the same cache key appears again, kubio compares the latest origin response fingerprint with the previous one. Matching fingerprints increase confidence. Any mismatch blocks automatic reuse.

kubio requires recent shadow validations with zero mismatches before auto reuse.

## Revalidation and Stale Recovery

When a verified cache entry becomes stale, kubio uses `If-None-Match` or `If-Modified-Since` to ask the origin whether the response changed. A `304 Not Modified` response refreshes the stored entry. A `200 OK` response replaces it if the response is still safe.

kubio serves stale during origin errors only when `Cache-Control: stale-if-error` or a route hint explicitly allows it, and only within the configured stale window.
