# Deployment

kubio v0.4.0 is designed for local-first and single-process deployment. The
released installer currently supports Linux x86_64.

## Install a Release Binary

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

Install to a specific directory:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_INSTALL_DIR=/usr/local/bin bash
```

Pin a release:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_VERSION=v0.4.0 bash
```

Install the HTTP/3 experimental artifact:

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | KUBIO_FLAVOR=http3-experimental bash
```

## Run as a Binary

```bash
kubio serve --to http://localhost:3000 --mode watch
```

Recommended rollout:

1. Start in watch mode.
2. Inspect dashboard route states and protected reasons.
3. Run shadow mode to validate repeated public responses.
4. Enable auto mode after validation.
5. Configure a panic file for rapid reuse disablement.

## Panic Switch

```bash
kubio serve --to http://localhost:3000 --mode auto --panic-file /tmp/kubio.disable
touch /tmp/kubio.disable
rm /tmp/kubio.disable
```

While the file exists, kubio keeps forwarding to origin and does not serve,
store, or promote reused responses. Removing the file restores normal
policy-controlled reuse.

## Updates

```bash
kubio update --check
kubio update
```

Update checks use public GitHub Release metadata. Disable best-effort ambient
notices:

```bash
KUBIO_UPDATE_CHECK=off kubio serve --to http://localhost:3000
kubio serve --no-update-check --to http://localhost:3000
```

## Storage

Use `storage.kind: disk` for process-local persistence across restarts. Disk
storage is still single-node and does not provide multi-instance shared cache
consistency.

## Docker

```bash
docker build -t kubio .
docker run --rm -p 8080:8080 -p 9900:9900 kubio serve --to http://host.docker.internal:3000
```
