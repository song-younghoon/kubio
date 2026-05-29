# Contributing

## Local Development

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

Run kubio locally:

```bash
cargo run -p kubio-cli -- serve --to http://localhost:3000
```

## Adding a Policy Rule

Policy rules live in `crates/kubio-policy`. Every rule must:

- Return a structured `DecisionReason`.
- Have unit tests.
- Preserve fail-open-to-origin behavior.
- Avoid exposing raw sensitive values.

If a rule is safety-critical, add an integration test or document the missing coverage.

## Adding Dashboard Fields

Dashboard data should come from observer snapshots, not direct hot-path state. New fields must avoid raw paths with user identifiers, query strings, request bodies, and sensitive header values.

## Pull Request Expectations

- Keep changes scoped.
- Include tests for behavior changes.
- Run formatting and tests before opening a PR.
- Update docs when user-facing behavior changes.
