# Deployment

v0.1.0 is designed for local-first and single-process deployment.

Run as a binary:

```bash
kubio serve --to http://localhost:3000 --mode watch
```

Run with Docker:

```bash
docker build -t kubio .
docker run --rm -p 8080:8080 -p 9900:9900 kubio serve --to http://host.docker.internal:3000
```

Recommended rollout:

1. Start in watch mode.
2. Inspect dashboard route states and protected reasons.
3. Run shadow mode to validate repeated public responses.
4. Enable auto mode after validation.
5. Configure a panic file for rapid reuse disablement.

v0.1.0 does not provide multi-instance shared cache consistency.
