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

Panic switch example:

```bash
kubio serve --to http://localhost:3000 --mode auto --panic-file /tmp/kubio.disable
touch /tmp/kubio.disable
rm /tmp/kubio.disable
```

While the file exists, kubio keeps forwarding to origin and does not serve, store, or promote reused responses. Removing the file restores normal policy-controlled reuse.

v0.1.0 does not provide multi-instance shared cache consistency.
