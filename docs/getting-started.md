# Getting Started

Start an origin:

```bash
python -m http.server 3000
```

Run kubio:

```bash
kubio serve --to http://localhost:3000
```

Send traffic through kubio:

```bash
curl http://localhost:8080
```

Open the dashboard:

```text
http://127.0.0.1:9900
```

kubio starts in watch mode. It observes requests and responses without reusing cached data.

To validate repeated responses without changing client-visible behavior:

```bash
kubio serve --to http://localhost:3000 --mode shadow
```

To enable safe automatic reuse:

```bash
kubio serve --to http://localhost:3000 --mode auto
```
