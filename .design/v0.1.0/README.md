# kubio v0.1.0 Design Index

Status: draft
Source: `PRD.md` v0.1 draft
Target release: `v0.1.0`

This directory turns the product requirements into implementation-facing designs and tasks for the first kubio release. The goal is a local-first Rust reverse proxy that observes API traffic by default, validates reuse through shadow checks, and only serves cached responses when conservative safety gates pass.

## Release Definition

kubio v0.1.0 is complete when a user can:

- Run `kubio serve --to http://localhost:3000`.
- Send HTTP/1.1 traffic through `0.0.0.0:8080`.
- See watch-mode route observations in a localhost dashboard.
- Enable `--mode shadow` to validate repeated public GET/HEAD responses without changing client-visible behavior.
- Enable `--mode auto` and get reuse only for safe, verified GET/HEAD 200 responses.
- Scrape Prometheus-compatible metrics.
- Understand every protection, bypass, store, and reuse decision from CLI/dashboard output.

## Design Documents

- [System Architecture](system-architecture.md)
  - Workspace layout, crate responsibilities, runtime topology, shared state, dependency choices, and fail-open behavior.
- [Request Lifecycle](request-lifecycle.md)
  - Detailed proxy flow for watch, shadow, and auto modes, including streaming, cache lookup, cache storage, and response finalization.
- [Policy and Safety](policy-and-safety.md)
  - Safety classifier, decision model, route clustering, cache keys, fingerprints, scoring, and promotion/demotion rules.
- [Observability and Dashboard](observability-dashboard.md)
  - In-memory observation model, metrics, event stream, dashboard APIs, UI pages, and redaction rules.
- [Testing and Release](testing-release.md)
  - Unit, integration, property, performance, security, and release verification design.
- [Implementation Tasks](tasks.md)
  - Milestone-by-milestone work breakdown with dependencies and acceptance checks.

## Cross-Cutting Constraints

- Safe default: unknown, risky, or failed paths go to origin.
- Privacy default: do not store sensitive header values, request bodies, or raw observation bodies.
- Local first: no hosted control plane, no required telemetry, dashboard binds to localhost.
- Explainability: every decision has machine-readable reasons and user-facing language.
- Bounded cardinality: metrics and in-memory state must avoid raw paths, query strings, and user-specific labels.
- Conservative v0.1.0 scope: HTTP/1.1 reverse proxy, in-memory store, local dashboard, Prometheus metrics, no distributed cache, no GraphQL/POST reuse.

## Milestone Map

- M0: Project skeleton
- M1: Basic reverse proxy
- M2: Observation
- M3: Safety classifier
- M4: Shadow validation
- M5: Safe auto reuse
- M6: Documentation and release hardening

Each milestone should leave the binary runnable and the safety model intact. Partially implemented optimization features must degrade to origin pass-through rather than changing responses unsafely.
