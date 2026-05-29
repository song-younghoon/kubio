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
kubio purge --all --admin-token "$KUBIO_ADMIN_TOKEN"
```

## Safety Defaults

kubio protects:

- Requests with `Authorization`.
- Requests with `Cookie`.
- Unsafe methods such as POST, PUT, PATCH, and DELETE.
- Responses with `Set-Cookie`.
- Responses with `Cache-Control: no-store` or `private`.
- Responses with `Cache-Control: no-cache` unless they can be revalidated with `ETag` or `Last-Modified`.
- Responses with `Vary: *` or unsupported `Vary` headers.
- Sensitive-looking routes such as `/me`, `/account`, `/login`, and `/admin`.

When kubio is unsure, it passes through to origin.

Configure `--panic-file /path/to/file` to immediately disable reuse while keeping origin pass-through active.

## Project Status

This repository is at v0.2.0 implementation stage. v0.2.0 is local-first and process-local:

- HTTP/1.1 reverse proxy.
- Local dashboard.
- Prometheus-style metrics.
- Configurable metrics path.
- In-memory observation store.
- In-memory or process-local disk cache store.
- Conditional revalidation with `ETag` and `Last-Modified`.
- Bounded stale-if-error when origin headers or route policy explicitly allow it.
- Route policy hints and query key hints.
- No hosted control plane.
- No required telemetry.
- No distributed cache.

v0.3.0 is currently design-stage and focuses on performance, HTTP/2, and guarded HTTP/3 support.

See [.design/v0.1.0](.design/v0.1.0), [.design/v0.2.0](.design/v0.2.0), [.design/v0.3.0](.design/v0.3.0), and [docs/safety-model.md](docs/safety-model.md).
