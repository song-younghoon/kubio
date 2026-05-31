# Header Sanitization and Store Contract

Status: implemented
Target release: `v0.5.2`

## Goals

v0.5.2 must separate two related but different concepts:

- **fingerprint headers**: headers used to prove whether two origin responses
  represent the same reusable response;
- **served headers**: headers replayed to downstream clients from a cached
  entry.

A header can be safe to ignore for fingerprinting and still unsafe or
misleading to replay from cache. `x-response-id` is the canonical example.

## Origin Miss Behavior

On an origin miss or canary pass-through, kubio should continue forwarding the
origin response headers as it does today, except for existing hop-by-hop and
kubio-managed headers.

This preserves origin behavior for requests that actually reach the origin.

## Store-Time Header Shape

When storing a cache entry, kubio should compute:

```text
origin headers
  -> fingerprint header set
  -> stored response header set
  -> suppressed-on-hit header names
```

The stored entry should include only headers that are safe to replay from cache.

Default `suppress_on_hit` class:

```text
X-Request-Id
X-Response-Id
X-Correlation-Id
X-Trace-Id
Request-Id
Response-Id
Correlation-Id
Traceparent
Tracestate
X-Amzn-Requestid
X-Amzn-Trace-Id
X-Cloud-Trace-Context
X-B3-*
CF-Ray
Fastly-Trace-Id
```

`Date` can be handled separately:

- preserve stored `Date` and add/update `Age`, or
- omit `Date` on cache hits if configured.

The default should prefer normal cache behavior: preserve origin `Date` and add
or update `Age` when possible.

## Cache Hit Behavior

On a fresh cache hit, kubio should:

1. return the stored safe headers;
2. avoid replaying suppressed one-shot metadata headers;
3. continue stripping hop-by-hop headers;
4. continue managing `Alt-Svc` through kubio's existing logic;
5. optionally add `Age`;
6. add debug headers only when debug headers are enabled.

Example:

Origin miss:

```text
X-Response-Id: res-a
X-Correlation-Id: corr-a
```

Cache hit:

```text
Age: 12
X-Kubio-Status: hit        # only when debug_headers is enabled
```

The hit should not include `res-a`.

## Config Shape

Suggested global config:

```yaml
policy:
  response_header_equivalence:
    enabled: true
    serve:
      strip_volatile_on_hit: true
      strip_verified_ignored_on_hit: true
      add_age: true
      preserve_date: true
    default_volatile:
      add: []
      block: []
```

Suggested route hint:

```yaml
routes:
  - match:
      method: GET
      path: /notice/{id}
    response_headers:
      verified_ignore:
        enabled: true
        allow: ["x-vendor-execution-id"]
      preserve_on_hit: []
      force_include: ["etag"]
```

Rules:

- global `block` removes a name from default volatile treatment and forces it
  back into fingerprinting;
- route `force_include` wins over global defaults;
- route `preserve_on_hit` may preserve a volatile header only if it is present
  in the stored safe header set;
- hard safety headers cannot be suppressed to make a response store-safe.

## Disk Metadata

Disk entries should record header policy metadata:

```rust
pub struct StoredHeaderPolicyMetadata {
    pub fingerprint_policy_version: u16,
    pub ignored_header_names: Vec<String>,
    pub suppressed_header_names: Vec<String>,
    pub response_header_equivalence_group: Option<String>,
}
```

Values are not stored in this metadata. Header names are bounded and lowercased.

## Compatibility With Existing Entries

Existing v0.5.1 disk entries do not have header policy metadata. v0.5.2 should
handle them conservatively:

- legacy entries may be served only through existing freshness and safety paths;
- hit-time volatile stripping should still run on legacy entries;
- canary or shadow comparison against a legacy fingerprint should treat missing
  fingerprint policy version as `legacy`;
- when a legacy entry is refreshed from origin, it should be rewritten with
  v0.5.2 metadata;
- if a legacy fingerprint mismatch cannot be interpreted safely, kubio should
  pass through to origin and refresh rather than serve.

## Privacy Contract

The following must not appear in snapshots, events, metrics, debug headers, CLI
JSON, or disk metadata:

- raw volatile header values;
- raw request IDs;
- raw response IDs;
- raw trace IDs;
- raw correlation IDs;
- authorization or cookie values;
- body content.

Allowed:

- lowercased header names;
- bounded classes;
- short hashes of equivalence groups;
- counts and rates.
