# Revalidation and Staleness

Status: design draft
Target release: `v0.2.0`

## Goals

Revalidation lets kubio safely continue using cached public responses after their freshness window. Stale-if-error lets kubio protect clients from short origin failures only when stale use is explicitly allowed.

The design must preserve this rule:

```text
A stale response is never served merely because it exists.
```

It must be revalidated successfully or allowed by stale-if-error.

## Freshness Model

Each stored entry has:

- `created_at`: when kubio stored the body.
- `fresh_until`: when kubio can serve without origin contact.
- `stale_until`: latest time kubio may serve during origin failure, if allowed.
- `must_revalidate`: true for `Cache-Control: no-cache` and similar directives.
- `validators`: `ETag`, `Last-Modified`, or both.
- `origin_freshness`: parsed bounded origin directives.
- `policy_freshness`: kubio freshness profile or route hint.

Freshness calculation:

```text
fresh_until = now + min(kubio_ttl, origin_max_age_if_present)
```

If `no-cache` is present:

```text
fresh_until = now
must_revalidate = true
```

`no-store` and `private` remain hard denies and are not stored.

## Validator Extraction

kubio should extract validators from safe origin responses:

- `ETag`
- `Last-Modified`

Validation:

- Header values must parse as valid header values.
- Values must be bounded by `policy.revalidation.max_validator_length`.
- Empty values are ignored.
- Weak ETags are allowed for conditional GET because the origin owns validator semantics.

If both validators exist, send both:

```http
If-None-Match: "<etag>"
If-Modified-Since: Wed, 21 Oct 2015 07:28:00 GMT
```

`If-None-Match` is preferred by HTTP semantics, but sending both is acceptable.

## Conditional Request Flow

When auto mode finds a stale or must-revalidate entry:

1. Re-run request hard denies.
2. Confirm the entry belongs to the route/key and has validators.
3. Build an origin request equivalent to the client request.
4. Add conditional headers from stored validators.
5. Forward to origin.
6. Interpret the response.

### 304 Not Modified

On `304`:

- Serve the stored body and status to the client.
- Merge allowed updated response headers from the 304 into stored headers.
- Recompute `fresh_until` and `stale_until`.
- Preserve the original body fingerprint.
- Record `RevalidationNotModified`.
- Emit a revalidation event.

Headers to merge include:

- `Cache-Control`
- `Expires`
- `ETag`
- `Last-Modified`
- `Vary`
- safe representation metadata such as `Content-Type`

Hop-by-hop headers and `Set-Cookie` must not be stored.

If the 304 introduces `Set-Cookie`, `private`, `no-store`, `Vary: *`, or unsupported `Vary`, kubio must treat the entry as invalid for reuse, purge or mark it unusable, and pass through safely. Because 304 normally has no body, kubio should return the origin 304 only if it can preserve client-visible behavior; otherwise it should refetch unconditionally.

Conservative implementation: on unsafe 304 metadata, perform an unconditional origin fetch.

### 200 OK

On `200`:

- Return the origin response to the client.
- Evaluate response policy as a new origin response.
- Replace the cached entry only if safe and bounded.
- Record `RevalidationModified`.

### 3xx, 4xx, and 5xx

For v0.2.0:

- `304` means not modified.
- `200` means modified or validator not honored.
- `5xx`, timeout, connection errors, and body-read failures are origin errors for stale-if-error evaluation.
- Other statuses are pass-through and should not serve stale unless explicitly included later.

## `Cache-Control: no-cache`

v0.1.0 protects `no-cache` responses from reuse. v0.2.0 changes this carefully:

- `no-cache` is storeable only if all other safety checks pass.
- `no-cache` entries require validators.
- `no-cache` entries must revalidate before every use.
- A `no-cache` entry without validator is observed but not stored for reuse.

Decision reason:

```text
NoCacheRequiresRevalidation
```

User-facing message:

```text
The origin allows storage but requires kubio to revalidate before reuse.
```

## Stale-If-Error Permission

Stale serving requires one of:

### Origin Permission

```http
Cache-Control: max-age=30, stale-if-error=300
```

Allowed stale window is the lesser of:

- origin `stale-if-error` value
- kubio global cap
- route hint cap, if present

### Route Hint Permission

```yaml
routes:
  - match:
      method: GET
      path: "/api/catalog"
    stale_if_error:
      enabled: true
      max_stale: "5m"
```

Route hint permission still requires a previously verified safe entry.

## Stale-If-Error Flow

```text
stale entry
  -> has previous safe fingerprint and route/key eligibility
  -> origin revalidation or refresh request fails
  -> stale-if-error permission exists
  -> stale age <= stale window
  -> serve stale body
```

Stale is not served when:

- Request now has Authorization, Cookie, Range, unsafe method, or body-on-GET signals.
- Entry has no fingerprint.
- Entry was never shadow validated.
- Entry is older than `stale_until`.
- Entry came from a response with `private` or `no-store`.
- Route/key has a shadow mismatch.
- Panic switch is active. Panic switch still disables reuse and stale serving.

## State Transitions

Entry state:

```text
Fresh
  -> RequiresRevalidation when fresh_until <= now
  -> Invalidated when hard deny metadata appears

RequiresRevalidation
  -> Fresh on 304
  -> Fresh on safe 200 replacement
  -> StaleServed on eligible origin error
  -> Miss/PassThrough otherwise

StaleServed
  -> RequiresRevalidation on next request
  -> Invalidated when stale_until <= now
```

Route state remains v0.1.0-compatible:

- `Auto` routes can use revalidation.
- `ShadowValidated` routes can record simulated revalidation outcomes but not serve.
- `Protected` routes cannot revalidate for reuse.

## Debug Headers

When `--debug-headers` is enabled:

```http
X-Kubio-Status: hit
X-Kubio-Status: miss
X-Kubio-Status: revalidated
X-Kubio-Status: stale
X-Kubio-Status: protected
X-Kubio-Status: bypass
```

Do not include validator values or route ids in response headers.

## Acceptance

- Stale ETag entry receives conditional request and 304 serves stored body.
- Stale Last-Modified entry receives conditional request and 304 serves stored body.
- Revalidation 200 replaces the cache entry when safe.
- `no-cache` entries are never served without contacting origin.
- Entries without validators pass through on stale requests.
- Stale-if-error serves stale only when origin or route permission exists.
- Panic switch disables stale serving.
- Existing protected traffic is still never reused.
