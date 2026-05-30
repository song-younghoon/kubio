# Getting Started

This guide starts from the released Linux x86_64 binary. To work from source,
use `cargo run -p kubio-cli -- serve --to http://localhost:3000` instead of the
installed `kubio` command.

## 1. Install

```bash
curl -fsSL https://raw.githubusercontent.com/song-younghoon/kubio/refs/heads/main/install.sh | bash
```

The installer downloads the release artifact, verifies it with `SHA256SUMS`, and
installs `kubio`. If it prints a `PATH` hint, add that directory to your shell
startup file or run kubio by its full path.

Confirm the install:

```bash
kubio --version
```

## 2. Start an Origin

```bash
python -m http.server 3000
```

## 3. Start kubio

```bash
kubio serve --to http://localhost:3000
```

kubio listens on `0.0.0.0:8080` for proxy traffic and `127.0.0.1:9900` for the
local dashboard by default.

## 4. Send Traffic

```bash
curl http://localhost:8080
```

Open the dashboard:

```text
http://127.0.0.1:9900
```

kubio starts in watch mode. It observes requests and responses without reusing
cached data.

## 5. Try Shadow and Auto Modes

Validate repeated responses without changing client-visible behavior:

```bash
kubio serve --to http://localhost:3000 --mode shadow
```

Enable safe automatic reuse after validation:

```bash
kubio serve --to http://localhost:3000 --mode auto
```

## 6. Check for Updates

```bash
kubio update --check
```

Install the latest stable release:

```bash
kubio update
```
