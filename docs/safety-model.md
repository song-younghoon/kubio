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

## Privacy

kubio does not store Authorization values, Cookie values, Set-Cookie values, request bodies, or raw response bodies in observation metadata. Metrics and dashboard APIs use route templates, counts, flags, and hashes.
