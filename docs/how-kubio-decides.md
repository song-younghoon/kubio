# How kubio Decides

kubio uses deterministic rules, not machine learning.

Every request starts with safety checks. kubio protects unsafe methods, authenticated requests, cookie-based requests, range requests, GET/HEAD requests with bodies, and sensitive-looking paths.

Every origin response is checked before storage or reuse. kubio does not store responses with `Set-Cookie`, `Cache-Control: no-store`, `private`, `no-cache`, `Vary: *`, unsupported `Vary` headers, non-200 statuses, missing fingerprints, or oversized bodies.

## Route States

- Watching: kubio is observing only.
- Candidate: repeated safe traffic was observed.
- Shadow validated: repeated responses matched in shadow validation.
- Auto: kubio may reuse fresh verified responses.
- Protected: kubio found a risk signal or mismatch.

## Shadow Validation

When the same cache key appears again, kubio compares the latest origin response fingerprint with the previous one. Matching fingerprints increase confidence. Any mismatch blocks automatic reuse.

v0.1.0 requires recent shadow validations with zero mismatches before auto reuse.
