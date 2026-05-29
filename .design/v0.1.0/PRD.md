# PRD: kubio

Document version: v0.1 draft
Product type: Open-source software
Primary implementation language: Rust
Initial release target: Local-first, zero-config API caching autopilot
Core philosophy: **minimal configuration, conservative automation, explainable safety, fail open to origin**

---

## 1. Product Summary

**kubio is an open-source reverse proxy that sits in front of an API server and automatically learns which responses are safe to reuse.**

Instead of requiring developers to manually configure cache keys, TTLs, invalidation rules, and HTTP caching headers, kubio observes real traffic, identifies safe reuse opportunities, validates them through shadow checks, and only then reuses responses for low-risk requests.

kubio is not positioned as a traditional cache server. It is better described as an:

```text
API response reuse autopilot
```

or:

```text
Safe caching layer for APIs
```

The core promise:

> Put kubio in front of your API. It learns which responses are safe to reuse, reduces repeated origin calls, and protects everything else.

---

## 2. Background and Problem

HTTP/API caching can dramatically reduce latency, infrastructure cost, and origin server load. However, in practice, caching is difficult to use safely.

Developers usually need to answer questions such as:

```text
Can this endpoint be cached?
How long should the response stay fresh?
Which query parameters affect the response?
Should Authorization or Cookie headers be part of the cache key?
What happens if the response contains Set-Cookie?
How should Vary be handled?
How do we avoid serving one user’s response to another user?
How do we invalidate cached data when the origin changes?
```

Most teams want the benefits of caching, but they do not want to become experts in HTTP caching semantics.

kubio addresses this gap by making caching decisions automatically, conservatively, and transparently.

The product assumption is:

> Many APIs contain repeated, stable, public responses that could be safely reused, but developers either do not cache them or avoid caching entirely because the risk of getting it wrong is too high.

kubio should make safe caching approachable without requiring deep caching knowledge.

---

## 3. Product Goals

### 3.1 Primary Goals

kubio v0.1 should:

```text
1. Run as a standalone reverse proxy in front of an origin API.
2. Require only an origin URL to start.
3. Start in a safe observation mode by default.
4. Observe request and response patterns without storing sensitive data.
5. Detect repeated requests and stable responses.
6. Classify requests as safe, unsafe, or unknown.
7. Use shadow validation to test whether a response would have been safe to reuse.
8. Automatically reuse only conservative, verified GET/HEAD responses.
9. Protect personalized, authenticated, cookie-based, or otherwise risky requests.
10. Provide a local dashboard explaining kubio’s decisions.
11. Expose metrics suitable for Prometheus-style monitoring.
12. Fail open to the origin server if anything goes wrong.
```

### 3.2 UX Goals

The ideal first-run experience should be:

```bash
kubio serve --to http://localhost:3000
```

Expected startup output:

```text
kubio is watching your API.

Origin: http://localhost:3000
Proxy:  http://0.0.0.0:8080
Mode:   Watch

Response reuse is not active yet.
kubio will learn which responses are safe to reuse.

Dashboard: http://127.0.0.1:9900
```

Users should not need to understand these concepts before using kubio:

```text
TTL
Cache-Control
Vary
ETag
surrogate keys
stale-while-revalidate
cache key normalization
invalidation graphs
```

kubio should expose higher-level concepts instead:

```text
Watching
Candidate
Auto
Protected
Bypassed
Freshness
Safety
Savings
```

### 3.3 Open Source Goals

kubio is not a commercial SaaS product. It is open-source software.

Therefore, v0.1 should be:

```text
local-first
self-hosted
transparent
hackable
documented
privacy-preserving by default
usable without an external control plane
```

There should be no required hosted service, no mandatory telemetry, and no external dashboard dependency.

---

## 4. Non-Goals

kubio v0.1 will not attempt to provide:

```text
Global CDN functionality
Automatic TLS certificate management
Kubernetes operator
Multi-region distributed cache
Complex cache invalidation graph
GraphQL POST response caching
LLM semantic caching
User-specific private cache
Database event integration
Full RFC-complete HTTP cache behavior
Hosted SaaS dashboard
Multi-tenant commercial control plane
```

kubio v0.1 should be deliberately narrow:

> Safe automatic reuse for simple, public, repeated GET/HEAD API responses.

---

## 5. Product Principles

### 5.1 Safe by Default

kubio should prefer missing an optimization over serving the wrong response.

Core rule:

```text
When unsure, pass through to origin.
```

kubio must not automatically reuse responses when there are strong personalization or sensitivity signals.

Examples of default-protected traffic:

```text
Requests with Authorization
Requests with Cookie
Responses with Set-Cookie
Responses with Cache-Control: no-store
Responses with Cache-Control: private
Unsafe methods such as POST, PUT, PATCH, DELETE
Responses with Vary: *
Routes that appear user-specific or sensitive
Routes with shadow validation mismatches
```

### 5.2 Zero-Config First

kubio should work with one required parameter:

```text
origin URL
```

Everything else should have conservative defaults.

Default command:

```bash
kubio serve --to http://localhost:3000
```

Optional advanced command:

```bash
kubio serve \
  --listen 0.0.0.0:8080 \
  --to http://localhost:3000 \
  --dashboard 127.0.0.1:9900 \
  --mode auto \
  --freshness balanced
```

### 5.3 Explainable Automation

kubio must be able to explain every decision.

Example explanation:

```text
GET /api/products

Status:
Auto reuse enabled

kubio’s reasoning:
- Request method is GET.
- No Authorization header was observed.
- No Cookie header was observed.
- The origin response does not set cookies.
- The response fingerprint has been stable.
- Shadow validation passed.
- The route has high repeated traffic.
```

For protected requests:

```text
GET /api/me

Status:
Protected

kubio’s reasoning:
- Authorization header is present.
- This route may return user-specific data.
- kubio will not reuse this response.
```

### 5.4 Fail Open to Origin

kubio must never make the origin API unavailable because the cache layer fails.

Required behavior:

```text
Cache store failure      → pass through to origin
Policy engine failure    → pass through to origin
Dashboard failure        → proxy continues serving traffic
Metrics failure          → proxy continues serving traffic
Config reload failure    → keep previous config or pass through safely
Internal uncertainty     → pass through to origin
```

### 5.5 Privacy by Design

kubio should avoid storing sensitive data unless absolutely required for caching a response.

By default, kubio should not store:

```text
Authorization values
Cookie values
Set-Cookie values
Request bodies
Raw response bodies in observation metadata
PII-like field values
Raw query strings in high-cardinality metric labels
```

For observation and decision-making, kubio should prefer:

```text
hashes
flags
counts
fingerprints
route templates
status classes
latency distributions
```

---

## 6. Target Users

### 6.1 Solo Developers and Small Teams

They want better API performance without learning advanced caching.

Needs:

```text
Run one command
See which endpoints are cacheable
Avoid dangerous caching mistakes
Get simple local visibility
```

### 6.2 Backend Engineers

They want to reduce origin load and improve latency.

Needs:

```text
Endpoint-level insights
Automatic candidate detection
Shadow validation
Safe auto mode
Override escape hatches
Metrics
```

### 6.3 Platform and SRE Engineers

They want a conservative optimization layer that can be deployed safely.

Needs:

```text
Fail-open behavior
Prometheus-compatible metrics
Structured logs
Config file support
Operational visibility
Panic switch
Clear safety model
```

### 6.4 Open Source Contributors

They want a modular Rust project with clear internals.

Needs:

```text
Well-separated crates
Documented policy engine
Testable decision rules
Good contribution guide
Safety-focused architecture
```

---

## 7. Core User-Facing Concepts

kubio should expose a small vocabulary.

### 7.1 Watching

kubio is observing traffic but not reusing responses.

```text
No response reuse.
No cache hits.
No behavior change for clients.
```

This is the default mode.

### 7.2 Candidate

kubio has found a route that may be safe to reuse, but it has not been promoted to automatic reuse yet.

```text
Repeated traffic detected.
Response appears stable.
More validation may be required.
```

### 7.3 Auto

kubio is automatically reusing verified responses for this route or cache key.

```text
Only safe, verified responses are reused.
TTL is conservative.
Risk signals still cause pass-through.
```

### 7.4 Protected

kubio has identified a request or route as risky and will not reuse responses.

Examples:

```text
User-specific route
Authenticated request
Cookie-based request
Sensitive path
Set-Cookie response
Private/no-store response
```

### 7.5 Bypassed

kubio passed the request through to origin due to policy, configuration, uncertainty, or internal failure.

---

## 8. Runtime Modes

kubio has three runtime modes.

```text
watch
shadow
auto
```

### 8.1 Watch Mode

Default mode.

Behavior:

```text
All requests go to origin.
No cached response is served to clients.
kubio records safe metadata.
kubio identifies candidate routes.
```

Use case:

```text
Initial deployment
Safety review
Traffic analysis
No-risk evaluation
```

### 8.2 Shadow Mode

Behavior:

```text
All client responses still come from origin.
kubio stores fingerprints for candidate responses.
When the same cache key appears again, kubio compares previous and current fingerprints.
kubio answers the question: “Would a cached response have been correct?”
```

Shadow mode builds confidence before real reuse.

Use case:

```text
Validation before enabling auto mode
Measuring cache prediction accuracy
Detecting unstable endpoints
```

### 8.3 Auto Mode

Behavior:

```text
kubio reuses responses only when safety gates and validation thresholds pass.
Risky requests are protected.
Unknown requests pass through.
```

Use case:

```text
Production optimization after observation and validation
```

---

## 9. Representative User Scenarios

### 9.1 Start kubio in Front of a Local API

Command:

```bash
kubio serve --to http://localhost:3000
```

Expected behavior:

```text
kubio listens on 0.0.0.0:8080.
kubio forwards traffic to http://localhost:3000.
kubio starts in watch mode.
No client-visible caching occurs.
The local dashboard is available on 127.0.0.1:9900.
```

Acceptance criteria:

```text
All requests are forwarded to origin.
Response status, headers, and body are preserved.
kubio records route-level metadata.
No cache hits occur in watch mode.
```

### 9.2 View Candidate Routes

Dashboard example:

```text
kubio is watching your API

Observed requests:        12,481
Candidate routes:         4
Protected routes:         7
Estimated origin savings: 31%
```

Route list:

```text
GET /api/products       Candidate      High estimated savings
GET /api/categories     Candidate      High estimated savings
GET /api/me             Protected      Authorization detected
POST /api/login         Protected      Unsafe method
```

### 9.3 Validate with Shadow Mode

Command:

```bash
kubio serve --to http://localhost:3000 --mode shadow
```

Expected behavior:

```text
Every client response still comes from origin.
kubio compares response fingerprints internally.
Routes with stable fingerprints gain confidence.
Routes with mismatches are demoted or protected.
```

Acceptance criteria:

```text
No cached response is served to clients in shadow mode.
Shadow matches and mismatches are recorded.
Routes with mismatches are not eligible for auto reuse.
Dashboard shows validation results.
```

### 9.4 Enable Safe Auto Mode

Command:

```bash
kubio serve --to http://localhost:3000 --mode auto
```

Expected behavior:

```text
Only verified, low-risk GET/HEAD responses may be reused.
All protected traffic passes through to origin.
kubio continues monitoring for safety changes.
```

Acceptance criteria:

```text
GET/HEAD public stable responses may be reused.
POST/PUT/PATCH/DELETE responses are never reused.
Requests with Authorization are protected.
Requests with Cookie are protected.
Responses with Set-Cookie are not stored.
Responses with no-store/private are not stored.
Shadow mismatch causes demotion.
Policy or store failure causes pass-through.
```

---

## 10. Functional Requirements

## 10.1 P0: Reverse Proxy

kubio must work as a basic HTTP reverse proxy.

Requirements:

```text
P0-RP-001: Accept inbound HTTP requests.
P0-RP-002: Forward requests to the configured origin.
P0-RP-003: Preserve method, path, query, body, and relevant headers.
P0-RP-004: Remove or correctly handle hop-by-hop headers.
P0-RP-005: Return origin status, headers, and body to the client.
P0-RP-006: Stream request and response bodies where possible.
P0-RP-007: Avoid buffering large bodies unless required for cache eligibility.
P0-RP-008: Return appropriate 502/504 responses on origin failure.
P0-RP-009: Support HTTP/1.1 for v0.1.
P0-RP-010: HTTP/2 support is desirable but not required for initial v0.1.
```

Recommended implementation direction:

```text
Rust
Tokio async runtime
hyper or axum-based HTTP server/proxy layer
tower middleware where useful
```

---

## 10.2 P0: Safety Classifier

kubio must classify each request and response using conservative safety rules.

Request signals:

```text
method
path
query presence
query parameter count
has_authorization
has_cookie
content_type
content_length
sensitive path keywords
```

Response signals:

```text
status_code
has_set_cookie
cache_control
vary
etag
last_modified
content_type
content_length
body_hash_available
```

Sensitive path keyword examples:

```text
/me
/user
/users
/account
/profile
/session
/login
/logout
/billing
/payment
/checkout
/admin
/token
/oauth
```

Requirements:

```text
P0-SAFE-001: Unsafe methods are not eligible for automatic reuse.
P0-SAFE-002: Requests with Authorization are protected by default.
P0-SAFE-003: Requests with Cookie are protected by default.
P0-SAFE-004: Responses with Set-Cookie are not stored.
P0-SAFE-005: Responses with Cache-Control: no-store are not stored.
P0-SAFE-006: Responses with Cache-Control: private are not stored in shared cache.
P0-SAFE-007: Responses with Vary: * are not automatically reused.
P0-SAFE-008: Sensitive-looking paths are protected or require stronger validation.
P0-SAFE-009: Classifier errors result in protection or pass-through.
```

---

## 10.3 P0: Observation Engine

kubio must collect metadata needed to understand traffic patterns.

Collected metadata:

```text
timestamp
route_id
cache_key_hash
method
status_code
latency_ms
response_size
body_fingerprint
has_authorization
has_cookie
has_set_cookie
cache_control_class
vary_class
decision
decision_reason
```

Data that must not be stored in observation metadata:

```text
Authorization header value
Cookie header value
Set-Cookie header value
Request body
Raw response body
PII-like field values
```

Requirements:

```text
P0-OBS-001: Record request counts by route.
P0-OBS-002: Record origin request counts by route.
P0-OBS-003: Record protected request counts by route.
P0-OBS-004: Record response status distribution.
P0-OBS-005: Record latency distribution.
P0-OBS-006: Record cache key repeat frequency.
P0-OBS-007: Track response fingerprint stability.
P0-OBS-008: Never expose sensitive header values in logs or dashboard.
```

---

## 10.4 P0: Route Clustering

kubio does not know application route definitions. It must infer route templates heuristically.

Examples:

```text
GET /api/products/123      → GET /api/products/{id}
GET /api/products/456      → GET /api/products/{id}
GET /api/users/018f...     → GET /api/users/{id}
GET /api/search?q=iphone   → GET /api/search
```

Basic normalization rules:

```text
Numeric path segment       → {id}
UUID-like segment          → {id}
ULID-like segment          → {id}
Long hex segment           → {id}
Other segment              → unchanged
```

Requirements:

```text
P0-ROUTE-001: route_id consists of method + normalized path template.
P0-ROUTE-002: Query parameters are not included in route_id.
P0-ROUTE-003: Query parameters are included in cache key.
P0-ROUTE-004: Route clustering must never panic.
P0-ROUTE-005: If clustering fails, raw path is used safely.
```

---

## 10.5 P0: Cache Key Generation

The cache key must be more specific than the route ID.

Default cache key components:

```text
method
scheme
authority
path
normalized query
selected Vary request headers
```

Query normalization:

```text
Sort query parameters by name.
Preserve original order for repeated parameter names.
Preserve all query parameters by default.
Do not remove tracking parameters automatically in v0.1.
```

Requirements:

```text
P0-KEY-001: Generate cache keys only for GET/HEAD requests.
P0-KEY-002: Normalize query parameter order.
P0-KEY-003: Preserve all query parameters by default.
P0-KEY-004: Include Vary-selected request headers in the cache key.
P0-KEY-005: If Vary cannot be safely handled, do not reuse.
P0-KEY-006: Do not expose raw cache keys containing sensitive values in metrics.
```

v0.1 may report query parameters that appear irrelevant, but it must not automatically remove them from the cache key.

---

## 10.6 P0: Response Fingerprinting

kubio needs response fingerprints to determine stability.

Fingerprint inputs:

```text
status_code
selected stable response headers
body hash
```

Volatile headers should be excluded:

```text
Date
Age
Server
Via
X-Request-Id
Traceparent
tracing headers
```

Requirements:

```text
P0-FP-001: Compute body hash using streaming where possible.
P0-FP-002: Avoid storing raw response body for observation.
P0-FP-003: Exclude volatile headers from fingerprint.
P0-FP-004: If response body exceeds max_fingerprint_body_size, skip candidate promotion or omit body hash.
P0-FP-005: Fingerprint failure makes the response ineligible for auto reuse.
```

Suggested default:

```yaml
policy:
  max_fingerprint_body_size: 2MiB
```

---

## 10.7 P0: Shadow Validation

Shadow validation is the core trust-building mechanism.

Process:

```text
1. A candidate request goes to origin.
2. kubio records the response fingerprint.
3. The same cache key appears again.
4. kubio sends the request to origin again.
5. kubio compares the new fingerprint with the previous fingerprint.
6. If they match, shadow_match increases.
7. If they differ, shadow_mismatch increases.
8. Routes with mismatches are demoted or protected.
```

Requirements:

```text
P0-SHADOW-001: Shadow mode never serves cached responses to clients.
P0-SHADOW-002: Track matches and mismatches by route and cache key.
P0-SHADOW-003: A mismatch prevents promotion to auto reuse.
P0-SHADOW-004: Dashboard shows shadow validation results.
P0-SHADOW-005: Auto mode may continue sampling origin responses for validation.
```

Suggested initial thresholds:

```yaml
policy:
  min_route_samples: 100
  min_key_repeats: 5
  min_shadow_validations: 20
  max_shadow_mismatch_rate: 0.001
```

Simpler v0.1 rule:

```text
A route is eligible for auto reuse only after 20 recent shadow validations with 0 mismatches.
```

---

## 10.8 P0: Safe Auto Reuse

In auto mode, kubio reuses only responses that pass strict conditions.

Required conditions:

```text
Method is GET or HEAD.
Status code is 200.
Request has no Authorization header.
Request has no Cookie header.
Response has no Set-Cookie header.
Response does not contain Cache-Control: no-store.
Response does not contain Cache-Control: private.
Response does not contain Vary: *.
Response body size is within max_object_size.
Shadow validation has passed.
Estimated benefit is meaningful.
```

Requirements:

```text
P0-AUTO-001: Run safety checks on every request, even in auto mode.
P0-AUTO-002: Reuse only fresh cache entries.
P0-AUTO-003: Expired entries result in origin pass-through.
P0-AUTO-004: Debug response headers are optional and disabled by default.
P0-AUTO-005: If debug headers are enabled, include X-Kubio-Status.
P0-AUTO-006: Any policy uncertainty results in origin pass-through.
```

Optional debug headers:

```http
X-Kubio-Status: hit
X-Kubio-Status: miss
X-Kubio-Status: protected
X-Kubio-Status: bypass
```

Freshness profiles:

```yaml
freshness_profiles:
  strict:
    max_ttl: 5s
  balanced:
    max_ttl: 30s
  relaxed:
    max_ttl: 120s
```

User-facing wording:

```text
strict   → High freshness
balanced → Balanced
relaxed  → Higher savings
```

---

## 10.9 P0: In-Memory Cache Store

v0.1 default storage is process-local memory.

Requirements:

```text
P0-STORE-001: Support TTL-based expiration.
P0-STORE-002: Support max total cache size.
P0-STORE-003: Support max object size.
P0-STORE-004: Track cache bytes and entry count.
P0-STORE-005: Evict entries when size limits are exceeded.
P0-STORE-006: Store failure results in origin pass-through.
```

Default config:

```yaml
storage:
  kind: memory
  max_size: 256MiB
  max_object_size: 1MiB
```

Cache entry fields:

```text
status
headers
body bytes
created_at
expires_at
fingerprint
route_id
cache_key_hash
```

---

## 10.10 P0: Local Dashboard

kubio should provide a local dashboard.

Default bind address:

```text
127.0.0.1:9900
```

Pages:

```text
/
  Overview

/routes
  Route list

/routes/:route_id
  Route detail

/events
  Recent policy events

/config
  Effective configuration, read-only
```

Overview fields:

```text
Observed requests
Origin requests
Reused responses
Protected requests
Bypassed requests
Candidate routes
Auto routes
Estimated savings
Actual reuse rate
Shadow matches
Shadow mismatches
p50 latency
p95 latency
```

Route detail fields:

```text
Route state
kubio’s explanation
Request count
Repeat rate
Status distribution
Fingerprint stability
Shadow validation result
Estimated benefit
Current freshness profile
Recent events
```

Requirements:

```text
P0-DASH-001: Dashboard binds to localhost by default.
P0-DASH-002: Dashboard exposes JSON APIs for the UI.
P0-DASH-003: Dashboard failure does not affect proxy traffic.
P0-DASH-004: Dashboard must not show sensitive header values.
P0-DASH-005: Public dashboard binding requires explicit configuration.
```

---

## 10.11 P0: Metrics

kubio must expose metrics suitable for Prometheus scraping.

Default endpoint:

```text
GET /metrics
```

Required metrics:

```text
kubio_requests_total
kubio_origin_requests_total
kubio_reused_responses_total
kubio_protected_requests_total
kubio_bypass_requests_total
kubio_shadow_matches_total
kubio_shadow_mismatches_total
kubio_cache_entries
kubio_cache_bytes
kubio_cache_evictions_total
kubio_request_duration_seconds
kubio_origin_duration_seconds
kubio_policy_decisions_total
```

Allowed labels:

```text
method
route_id
decision
status_class
```

Forbidden labels:

```text
raw path
query string
user ID
header value
Authorization value
Cookie value
IP address by default
```

Metric cardinality must remain bounded.

---

## 10.12 P0: CLI

kubio should provide a simple CLI.

### `serve`

```bash
kubio serve --to http://localhost:3000
```

Options:

```text
--to <URL>                 origin URL
--listen <ADDR>            default: 0.0.0.0:8080
--dashboard <ADDR>         default: 127.0.0.1:9900
--mode <watch|shadow|auto> default: watch
--config <PATH>            optional config file
--freshness <strict|balanced|relaxed>
--debug-headers            add X-Kubio-* response headers
```

### `routes`

```bash
kubio routes
```

Shows observed route summaries.

### `explain`

```bash
kubio explain "GET /api/products"
```

Shows kubio’s decision and reasoning for a route.

### `doctor`

```bash
kubio doctor
```

Checks:

```text
Config parsing
Origin connectivity
Dashboard binding
Storage configuration
Metrics endpoint
Panic switch status
```

### `purge`

```bash
kubio purge --all
kubio purge --route "GET /api/products"
```

For v0.1, purge may operate through a local admin API or process-local control channel.

---

## 11. Configuration

Configuration file is optional.

Example:

```yaml
version: 1

server:
  listen: "0.0.0.0:8080"

origin: "http://localhost:3000"

mode: "watch"
freshness: "balanced"

dashboard:
  enabled: true
  listen: "127.0.0.1:9900"

policy:
  respect_origin_headers: true
  cache_methods: ["GET", "HEAD"]
  auto_status_codes: [200]
  protect_authorization: true
  protect_cookies: true
  protect_set_cookie: true
  max_object_size: "1MiB"
  max_fingerprint_body_size: "2MiB"
  min_route_samples: 100
  min_key_repeats: 5
  min_shadow_validations: 20
  max_shadow_mismatch_rate: 0.001

storage:
  kind: "memory"
  max_size: "256MiB"

observability:
  metrics: true
  metrics_path: "/metrics"
  tracing: true
```

Configuration principles:

```text
The default config should be safe.
Advanced options should be documented but not required.
Config should be human-readable.
Invalid config should fail clearly before serving traffic.
```

---

## 12. Policy Decision Model

kubio should model decisions explicitly.

Suggested Rust enum:

```rust
enum Decision {
    Reuse,
    StoreOnly,
    ObserveOnly,
    Protect,
    Bypass,
}
```

Meanings:

```text
Reuse
  Return a cached response to the client.

StoreOnly
  Return the origin response to the client and store it for future reuse.

ObserveOnly
  Return the origin response and record only metadata/fingerprint.

Protect
  Treat request/route as risky and exclude from reuse.

Bypass
  Pass through due to policy, config, error, or uncertainty.
```

Decision reasons:

```rust
enum DecisionReason {
    MethodNotCacheable,
    HasAuthorization,
    HasCookie,
    HasSetCookie,
    CacheControlNoStore,
    CacheControlPrivate,
    VaryUnsupported,
    ShadowMismatch,
    InsufficientSamples,
    LowEstimatedBenefit,
    ObjectTooLarge,
    PolicyError,
    StoreError,
    ReusableAndFresh,
}
```

Every decision should have at least one reason.

---

## 13. Cacheability Scoring

v0.1 should use deterministic scoring, not machine learning.

Example scoring model:

```text
base score = 0

+30 GET/HEAD
+20 no Authorization
+20 no Cookie
+20 no Set-Cookie
+20 stable fingerprint
+20 high repeat rate
+10 simple query
+10 origin headers allow storage

-100 no-store
-100 private
-100 Authorization present
-80 Cookie present
-80 Set-Cookie present
-80 unsupported Vary
-60 shadow mismatch
-40 high query cardinality
-30 sensitive path
```

State mapping:

```text
score < 0          → Protected
0..49              → Observing
50..79             → Candidate
80+ + shadow pass  → Auto
```

Hard deny rules must override score:

```text
Cache-Control: no-store
Cache-Control: private
Authorization
Cookie
Set-Cookie
Unsafe method
Vary: *
Shadow mismatch
```

---

## 14. Architecture

Suggested workspace structure:

```text
kubio/
  crates/
    kubio-cli/
    kubio-proxy/
    kubio-core/
    kubio-policy/
    kubio-store/
    kubio-observe/
    kubio-dashboard/
    kubio-telemetry/
```

### 14.1 kubio-cli

Responsibilities:

```text
CLI parsing
Config loading
Process lifecycle
Subcommands
Startup output
```

### 14.2 kubio-proxy

Responsibilities:

```text
HTTP server
Origin client
Request forwarding
Response streaming
Body hashing/buffering bridge
Proxy error handling
```

### 14.3 kubio-core

Responsibilities:

```text
Shared types
RouteId
CacheKey
Decision
DecisionReason
ResponseFingerprint
Time utilities
```

### 14.4 kubio-policy

Responsibilities:

```text
Safety classifier
Cacheability scoring
Freshness selection
Decision explanation
Promotion/demotion rules
```

### 14.5 kubio-store

Responsibilities:

```text
CacheStore trait
MemoryStore implementation
TTL expiration
Size accounting
Eviction
Purge
```

Suggested trait:

```rust
#[async_trait::async_trait]
pub trait CacheStore: Send + Sync {
    async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, StoreError>;
    async fn put(&self, key: CacheKey, entry: CacheEntry) -> Result<(), StoreError>;
    async fn purge(&self, selector: PurgeSelector) -> Result<PurgeResult, StoreError>;
}
```

### 14.6 kubio-observe

Responsibilities:

```text
Route statistics
Fingerprint history
Shadow validation records
Candidate detection
Event stream
```

### 14.7 kubio-dashboard

Responsibilities:

```text
Local UI
Read-only APIs
Route detail APIs
Admin purge endpoint
Config view
```

### 14.8 kubio-telemetry

Responsibilities:

```text
Metrics
Structured logs
Tracing integration
Log redaction
```

---

## 15. Data Model Draft

### 15.1 RouteId

```rust
struct RouteId {
    method: Method,
    template: String,
}
```

Example:

```text
GET /api/products/{id}
```

### 15.2 CacheKey

```rust
struct CacheKey {
    method: Method,
    scheme: String,
    authority: String,
    path: String,
    normalized_query: String,
    vary_headers: Vec<(String, String)>,
}
```

### 15.3 SafetySignals

```rust
struct SafetySignals {
    has_authorization: bool,
    has_cookie: bool,
    has_set_cookie: bool,
    cache_control: CacheControlClass,
    vary: VaryClass,
    sensitive_path_score: u8,
    method_cacheable: bool,
    status_cacheable: bool,
}
```

### 15.4 ResponseFingerprint

```rust
struct ResponseFingerprint {
    status: StatusCode,
    header_hash: Hash,
    body_hash: Option<Hash>,
}
```

### 15.5 RouteStats

```rust
struct RouteStats {
    route_id: RouteId,
    request_count: u64,
    origin_count: u64,
    reuse_count: u64,
    protected_count: u64,
    shadow_matches: u64,
    shadow_mismatches: u64,
    avg_latency_ms: f64,
    p95_latency_ms: f64,
    estimated_reuse_rate: f64,
    state: RouteState,
}
```

### 15.6 CacheEntry

```rust
struct CacheEntry {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
    created_at: Instant,
    expires_at: Instant,
    fingerprint: ResponseFingerprint,
}
```

---

## 16. HTTP Semantics

kubio v0.1 does not need to be a complete HTTP cache implementation, but it must respect safety-critical HTTP semantics.

Required handling:

```text
Cache-Control: no-store
Cache-Control: private
Cache-Control: no-cache
Vary
Authorization
Cookie
Set-Cookie
ETag
Last-Modified
Age
Date
```

v0.1 behavior:

```text
Cache-Control: no-store
  Do not store.

Cache-Control: private
  Do not store in kubio’s shared cache.

Cache-Control: no-cache
  Do not automatically reuse in v0.1.
  Revalidation may be added later.

Vary: *
  Do not reuse.

Vary: Accept-Encoding
  Include Accept-Encoding in cache key.

Authorization
  Protect by default, even if origin headers might allow caching.

Cookie
  Protect by default.

Set-Cookie
  Do not store.

ETag / Last-Modified
  Observe only in v0.1.
  Conditional revalidation is P1.

Age / Date
  Exclude from fingerprint or handle separately.
```

---

## 17. Security and Privacy Requirements

kubio is in the request path, so conservative security behavior is mandatory.

Requirements:

```text
SEC-001: Never store Authorization header values.
SEC-002: Never store Cookie header values in observation metadata.
SEC-003: Never store Set-Cookie header values in observation metadata.
SEC-004: Never store request bodies for observation.
SEC-005: Store response bodies only when necessary for cache entries.
SEC-006: Use body hashes for observation and shadow validation.
SEC-007: Dashboard binds to localhost by default.
SEC-008: Public dashboard binding requires explicit configuration.
SEC-009: Public dashboard binding should require an admin token.
SEC-010: Provide a panic switch to disable reuse immediately.
SEC-011: Logs must redact sensitive headers.
SEC-012: Metrics must avoid high-cardinality or sensitive labels.
```

Panic switch example:

```bash
kubio serve \
  --to http://localhost:3000 \
  --mode auto \
  --panic-file /tmp/kubio.disable
```

If the file exists:

```text
All requests pass through to origin.
No cached responses are served.
Observation may continue if safe.
```

---

## 18. Events

kubio should emit explainable events.

Event examples:

```text
route_candidate_detected
route_promoted_to_shadow
route_promoted_to_auto
route_demoted_due_to_shadow_mismatch
request_protected_due_to_authorization
request_protected_due_to_cookie
response_not_stored_due_to_no_store
response_not_stored_due_to_private
cache_entry_evicted
store_error_fail_open
panic_switch_enabled
panic_switch_disabled
```

Example event payload:

```json
{
  "event": "route_promoted_to_auto",
  "route_id": "GET /api/products",
  "reason": [
    "no authorization",
    "no cookies",
    "stable fingerprint",
    "20 shadow validations passed"
  ],
  "freshness": "balanced"
}
```

---

## 19. Dashboard Language

kubio should avoid low-level cache terminology in the main UI.

Preferred language mapping:

```text
cache hit        → reused
cache miss       → sent to origin
bypass           → passed through
not cacheable    → protected
TTL              → freshness
invalidation     → new data detected
fingerprint      → response pattern
```

Example route detail:

```text
GET /api/products

Status
Auto reuse enabled

Impact
Origin requests reduced by 64%
p95 latency improved from 312ms to 48ms

kubio’s reasoning
- This request is called without login information.
- No cookies were observed.
- The response pattern is stable.
- kubio validated this route 24 times with real traffic.
- No recent mismatches were detected.

Freshness
Balanced
```

Protected route example:

```text
GET /api/me

Status
Protected

kubio’s reasoning
- Authorization header was observed.
- This route may return user-specific data.
- kubio will not reuse this response.
```

---

## 20. Performance Requirements

Initial v0.1 targets:

```text
Pass-through p95 overhead: <= 5ms in local 100 RPS test
Cache-hit p95 overhead: <= 2ms in local 100 RPS test
Memory store lookup: expected O(1)
Dashboard polling must not block proxy hot path
Metrics collection must not block proxy hot path
Large responses should stream unless eligible and small enough for storage
```

Load test scenarios:

```text
100 RPS, 10 minutes, 1KiB JSON responses
500 RPS, 10 minutes, 10KiB JSON responses
100 RPS, 10 minutes, 1MiB responses
Mixed traffic: 70% GET, 20% POST, 10% authenticated GET
```

---

## 21. Testing Requirements

### 21.1 Unit Tests

Required unit test areas:

```text
Cache-Control parsing
Vary handling
Authorization protection
Cookie protection
Set-Cookie protection
Query normalization
Route clustering
Fingerprint generation
Decision reasons
Freshness selection
Sensitive path detection
```

### 21.2 Integration Tests

Use a test origin server and run traffic through kubio.

Required scenarios:

```text
Stable public GET response → reusable in auto mode
GET with Authorization → always origin
GET with Cookie → always origin
Response with Set-Cookie → not stored
Response with Cache-Control: no-store → not stored
Response with Cache-Control: private → not stored
Response with Vary: Accept-Encoding → key includes Accept-Encoding
Response with Vary: * → not reused
POST request → never reused
Shadow mismatch → route demoted
Store error → pass-through
Panic switch → no reuse
```

### 21.3 Property Tests

Suggested property tests:

```text
Query normalization is stable across parameter ordering.
Cache keys differ when relevant Vary header values differ.
Route clustering never panics for arbitrary paths.
Sensitive values are never emitted in metrics labels.
```

### 21.4 Performance Tests

Suggested benchmarks:

```text
Pass-through overhead
Cache-hit overhead
Memory store eviction
Large response streaming
Dashboard polling under traffic
Metrics endpoint under traffic
```

---

## 22. Open Source Requirements

### 22.1 Repository Structure

Suggested repository:

```text
github.com/song-younghoon/kubio
```

Initial files:

```text
README.md
LICENSE
CONTRIBUTING.md
CODE_OF_CONDUCT.md
SECURITY.md
docs/
examples/
crates/
.github/workflows/ci.yml
```

### 22.2 License

Recommended default:

```text
Apache-2.0
```

Rationale:

```text
Permissive
Friendly to infrastructure adoption
Common in Rust and cloud-native ecosystems
Includes explicit patent grant
```

Alternative to consider:

```text
MPL-2.0
```

MPL-2.0 may be considered if the project wants file-level copyleft for modifications to kubio itself.

PRD default:

```text
License: Apache-2.0
```

### 22.3 Contribution Model

Required documents:

```text
CONTRIBUTING.md
  Local development setup
  Test commands
  Coding style
  How to add a policy rule
  How to add dashboard fields

SECURITY.md
  Vulnerability reporting process
  Supported versions
  Disclosure policy

docs/safety-model.md
  What kubio never reuses
  How decisions are made
  Known limitations
```

### 22.4 CI Requirements

CI should run:

```text
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test --workspace
cargo deny check
cargo audit
integration tests
Docker image build
```

---

## 23. Release Milestones

### M0: Project Skeleton

Scope:

```text
Rust workspace
CLI skeleton
Config loading
Logging
Basic README
CI
```

Completion criteria:

```text
cargo test --workspace passes
kubio --help works
CI runs on pull requests
```

### M1: Basic Reverse Proxy

Scope:

```text
HTTP server
Origin forwarding
Streaming response
Error handling
```

Completion criteria:

```text
kubio serve --to http://localhost:3000
```

works as a local reverse proxy.

### M2: Observation

Scope:

```text
Route clustering
Metadata collection
Response fingerprinting
In-memory stats
```

Completion criteria:

```text
Dashboard/API shows request count and latency by route.
```

### M3: Safety Classifier

Scope:

```text
Authorization protection
Cookie protection
Set-Cookie protection
Cache-Control handling
Vary handling
Decision reasons
```

Completion criteria:

```text
Risky requests are marked Protected with explanations.
```

### M4: Shadow Validation

Scope:

```text
Fingerprint history
Match/mismatch tracking
Candidate promotion/demotion
Dashboard visibility
```

Completion criteria:

```text
Stable endpoints become candidates.
Unstable endpoints are excluded from auto reuse.
```

### M5: Safe Auto

Scope:

```text
Memory cache store
TTL/freshness profiles
Cache hit/miss behavior
Panic switch
Debug headers
```

Completion criteria:

```text
--mode auto reuses only safe GET/HEAD 200 responses.
Protected requests always go to origin.
```

### M6: v0.1 Release

Scope:

```text
Documentation
Examples
Dockerfile
Release binary
Security policy
Safety model document
```

Completion criteria:

```text
A new user can run a local demo from the README in under 5 minutes.
```

---

## 24. README First Example

The first example should be very simple.

```bash
# Start your app
python -m http.server 3000

# Put kubio in front of it
kubio serve --to http://localhost:3000

# Send traffic through kubio
curl http://localhost:8080
```

README copy:

```text
kubio starts in Watch mode.
It will not reuse responses until you explicitly enable Auto mode.
```

Auto mode example:

```bash
kubio serve --to http://localhost:3000 --mode auto
```

---

## 25. Documentation Structure

Required docs:

```text
docs/getting-started.md
docs/configuration.md
docs/how-kubio-decides.md
docs/safety-model.md
docs/metrics.md
docs/deployment.md
docs/development.md
docs/roadmap.md
```

`docs/how-kubio-decides.md` must explain:

```text
Protected conditions
Candidate conditions
Shadow validation
Auto promotion
Demotion
Known limitations
```

---

## 26. Known Limitations for v0.1

kubio v0.1 should clearly document its limitations.

```text
kubio v0.1 is not a complete HTTP cache implementation.
kubio v0.1 does not automatically reuse POST, GraphQL, or LLM responses.
kubio v0.1 does not support complex invalidation.
kubio v0.1 does not support distributed cache consistency.
kubio v0.1 does not support multi-instance shared cache by default.
kubio v0.1 does not automatically remove query parameters from cache keys.
kubio v0.1 protects authenticated requests by default, even if origin headers might technically allow caching.
```

Preferred framing:

> kubio prefers a missed optimization over a wrong response.

---

## 27. Success Criteria

### 27.1 Functional Success

v0.1 is successful if:

```text
kubio can run as a reverse proxy with one command.
Watch mode observes traffic without behavior change.
Dashboard shows route-level status and reasoning.
Safety classifier protects Authorization, Cookie, Set-Cookie, no-store, and private responses.
Shadow validation distinguishes stable and unstable endpoints.
Auto mode reuses only safe GET/HEAD 200 responses.
Prometheus-compatible metrics are available.
README demo works locally.
```

### 27.2 Quantitative Success

Initial target metrics:

```text
Pass-through p95 overhead <= 5ms
Cache-hit p95 overhead <= 2ms
Default memory usage <= 100MiB at idle
Zero raw request bodies stored in watch mode
Zero sensitive header values exposed in logs/metrics/dashboard
Safety integration tests pass 100%
```

### 27.3 Product Success

kubio succeeds if:

```text
Users can try it without learning HTTP caching.
Users can understand why a route is protected or reused.
Contributors can add policy rules safely.
The default behavior is conservative enough for real services.
```

---

## 28. v0.2+ Roadmap Candidates

Potential future features:

```text
Query parameter intelligence
  Detect irrelevant parameters such as utm_source or fbclid.
  Validate through shadow mode before ignoring them.

Conditional revalidation
  ETag / If-None-Match
  Last-Modified / If-Modified-Since

stale-if-error
  Serve recently verified responses during origin failures.

Disk store
  Local persistent cache.

Redis-compatible store
  Shared cache across processes or instances.

Mutation-aware bypass
  Observe POST/PATCH/DELETE and temporarily bypass related GET routes.

Route hints
  Allow origin responses to provide kubio-specific hints.

Explicit policy file
  Allow route-level allow/protect overrides.

Caddy module
  Reuse kubio’s policy engine inside Caddy.

Kubernetes deployment guide
  Sidecar and gateway deployment examples.

GraphQL-safe mode
  Detect operationName and variables, but require explicit opt-in.

LLM response cache
  Future separate feature, not part of v0.1.
```

---

## 29. Final Product Definition

kubio v0.1 is defined as:

> An open-source API response reuse autopilot written in Rust. It runs as a reverse proxy, observes traffic by default, validates safe reuse through shadow checks, and automatically reuses only conservative, verified GET/HEAD responses while protecting risky requests.

The most important product constraints are:

```text
Safety before speed.
Explanation before magic.
Defaults before configuration.
Pass-through before wrong response.
Open-source transparency before hidden intelligence.
```

The initial implementation should focus on:

```text
Standalone Rust binary
Tokio-based reverse proxy
Watch mode by default
Local dashboard
Prometheus-compatible metrics
Safety classifier
Shadow validation
Memory cache
Safe auto mode
Apache-2.0 license
```

kubio should become the caching layer for developers who want the benefits of caching without having to become caching experts.
