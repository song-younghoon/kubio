# kubio

kubio is an open-source reverse proxy that learns which API responses are safe to reuse. It starts in watch mode, protects risky traffic by default, validates repeated responses through shadow checks, and only reuses conservative GET/HEAD responses in auto mode.

## Quick Start

Start a local origin:

```bash
python -m http.server 3000
```

Run kubio in front of it:

```bash
cargo run -p kubio-cli -- serve --to http://localhost:3000
```

Send traffic through kubio:

```bash
curl http://localhost:8080
```

kubio starts in Watch mode. It does not reuse responses until you explicitly enable Auto mode.

Dashboard:

```text
http://127.0.0.1:9900
```

Metrics:

```text
http://127.0.0.1:9900/metrics
```

## Common Commands

```bash
kubio serve --to http://localhost:3000
kubio serve --to http://localhost:3000 --mode shadow
kubio serve --to http://localhost:3000 --mode auto
kubio routes
kubio explain "GET /api/products"
kubio doctor --to http://localhost:3000
kubio purge --all
```

## Safety Defaults

kubio protects:

- Requests with `Authorization`.
- Requests with `Cookie`.
- Unsafe methods such as POST, PUT, PATCH, and DELETE.
- Responses with `Set-Cookie`.
- Responses with `Cache-Control: no-store`, `private`, or `no-cache`.
- Responses with `Vary: *` or unsupported `Vary` headers.
- Sensitive-looking routes such as `/me`, `/account`, `/login`, and `/admin`.

When kubio is unsure, it passes through to origin.

## Project Status

This repository is at v0.1.0 implementation stage. v0.1.0 is local-first and process-local:

- HTTP/1.1 reverse proxy.
- Local dashboard.
- Prometheus-style metrics.
- In-memory observation and cache store.
- No hosted control plane.
- No required telemetry.
- No distributed cache.

See [.design/v0.1.0](.design/v0.1.0) and [docs/safety-model.md](docs/safety-model.md).
