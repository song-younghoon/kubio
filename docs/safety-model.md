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

Precision adaptive reuse can compact query keys only after fingerprint proof and
explicit enablement. Sensitive query names such as `token`, `session`, `jwt`,
`api_key`, `secret`, `signature`, and `code` are not automatic ignore
candidates. Canary or shadow mismatches place the affected route or key group
in cooldown and purge affected entries.

Response-header equivalence ignores only response metadata that does not define
the representation, such as `x-response-id` and trace IDs. It does not override
`Set-Cookie`, `Cache-Control`, `Vary`, validators, representation headers,
Authorization, Cookie, sensitive paths, panic switch, or mismatch handling.
Cache hits strip one-shot volatile response identifiers by default.

## Runtime Reload Safety

Reload is an atomic commit for behavioral config. kubio validates the candidate
file, rejects any restart-required diff, reconciles observer/cache state, then
publishes a new generation. If parsing, validation, diff classification, or
purge reconciliation fails, the active generation is unchanged.

Route hint removals or changes demote affected route evidence and purge affected
entries. Global policy compatibility changes use a conservative all-route purge
and demotion. Reload API responses, metrics, events, and dashboard HTML use
redacted config and bounded labels only.

## Privacy

kubio does not store Authorization values, Cookie values, Set-Cookie values,
volatile response header values, request bodies, or raw response bodies in
observation metadata. Metrics and dashboard APIs use route templates, counts,
flags, names, and hashes.
