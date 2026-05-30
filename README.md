# kubio

kubio is a local-first reverse proxy that learns which API responses are safe to
reuse. It starts in watch mode, protects risky traffic by default, validates
repeated responses through shadow checks, and only reuses conservative GET/HEAD
responses after you opt in.

Use kubio when you want a cautious API response reuse layer in front of an
origin service without a hosted control plane or required telemetry.

## Install

v0.4.0 supports released binaries for Linux x86_64.

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

The installer downloads a GitHub Release artifact, verifies it with
`SHA256SUMS`, installs `kubio`, and prints a `PATH` hint if needed. It does not
build from source.

Common install variants:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_VERSION=v0.4.0 bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR=/usr/local/bin bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_FLAVOR=http3-experimental bash
```

## Quick Start

Start a local origin:

```bash
python -m http.server 3000
```

Run kubio in front of it:

```bash
kubio serve --to http://localhost:3000
```

Send traffic through kubio:

```bash
curl http://localhost:8080
```

kubio starts in Watch mode. It observes traffic but does not reuse responses
until you choose Shadow or Auto mode.

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
kubio update --check
kubio update
```

## Update

Check whether a newer stable release exists:

```bash
kubio update --check
```

Install the latest stable release:

```bash
kubio update
```

Update checks use public GitHub Release metadata only. They do not send route,
origin, cache, dashboard, request, or config data. Disable best-effort ambient
notices with:

```bash
KUBIO_UPDATE_CHECK=off kubio serve --to http://localhost:3000
kubio serve --no-update-check --to http://localhost:3000
```

## Safety Defaults

kubio protects:

- Requests with `Authorization`.
- Requests with `Cookie`.
- Unsafe methods such as POST, PUT, PATCH, and DELETE.
- Responses with `Set-Cookie`.
- Responses with `Cache-Control: no-store` or `private`.
- Responses with `Cache-Control: no-cache` unless they can be revalidated with
  `ETag` or `Last-Modified`.
- Responses with `Vary: *` or unsupported `Vary` headers.
- Sensitive-looking routes such as `/me`, `/account`, `/login`, and `/admin`.

When kubio is unsure, it passes through to origin.

Configure `--panic-file /path/to/file` to immediately disable reuse while
keeping origin pass-through active.

## Development

Run from a checkout:

```bash
cargo run -p kubio-cli -- serve --to http://localhost:3000
```

Build the standard binary:

```bash
cargo build --release -p kubio-cli
```

Build the HTTP/3 experimental binary:

```bash
cargo build --release -p kubio-cli --features experimental-http3
```

## Project Status

kubio remains local-first and process-local:

- HTTP/1.1 reverse proxy.
- HTTP/2 downstream support through explicit h2c prior knowledge or TLS ALPN.
- HTTP/2 upstream support through reqwest.
- Experimental HTTP/3 support through the `experimental-http3` feature and a
  separate release artifact.
- Local dashboard and Prometheus-style metrics.
- In-memory or process-local disk cache store.
- Conditional revalidation with `ETag` and `Last-Modified`.
- Bounded stale-if-error when origin headers or route policy explicitly allow
  it.
- Route policy hints and query key hints.
- No hosted control plane.
- No required telemetry.
- No distributed cache.

## Docs

- [Getting Started](docs/getting-started.md)
- [Deployment](docs/deployment.md)
- [Install and Update](docs/install-update.md)
- [Configuration](docs/configuration.md)
- [Metrics](docs/metrics.md)
- [Safety Model](docs/safety-model.md)
- [Roadmap](docs/roadmap.md)
- [v0.4.0 Design](.design/v0.4.0)
