# Safety Model

kubio prefers a missed optimization over a wrong response.

## Never Reused by Default

- Requests with `Authorization`.
- Requests with `Cookie`.
- POST, PUT, PATCH, DELETE, and other unsafe methods.
- Range requests.
- GET/HEAD requests with bodies.
- Sensitive-looking paths such as `/me`, `/account`, `/login`, and `/admin`.
- Responses with `Set-Cookie`.
- Responses with `Cache-Control: no-store`.
- Responses with `Cache-Control: private`.
- Responses with `Cache-Control: no-cache` and no usable validator.
- Responses with `Vary: *`.
- Responses with unsupported `Vary` headers.
- Responses that fail fingerprinting.
- Responses with shadow validation mismatches.

## Fail Open

Policy errors, store errors, dashboard errors, metrics errors, and internal uncertainty pass through to origin. Origin failures return gateway errors rather than cached data unless stale-if-error is explicitly allowed by origin headers or route policy.

## Adaptive Reuse

Adaptive reuse can improve hit rate only after hard checks pass. Exact-key
validation, origin-public headers, route hints, and public-object cardinality do
not override Authorization, Cookie, Set-Cookie, private/no-store, unsupported
Vary, sensitive paths, panic switch, or shadow mismatches.

Public object evidence is route-level, but cached responses remain exact-key
objects. `/notice/1` and `/notice/2` may share confidence while staying separate
cache entries.

## Privacy

kubio does not store Authorization values, Cookie values, Set-Cookie values, request bodies, or raw response bodies in observation metadata. Metrics and dashboard APIs use route templates, counts, flags, and hashes.
