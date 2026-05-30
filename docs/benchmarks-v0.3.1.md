# v0.3.1 Benchmarks

Status: local smoke baseline captured on 2026-05-30.

Command shape:

```bash
cargo run -p kubio-bench -- --requests 5 --protocol h1 --output json
cargo run -p kubio-bench -- --requests 5 --protocol h2 --output json
cargo run -p kubio-bench --features experimental-http3 -- --requests 5 --protocol h3 --output json
```

Local smoke results:

| Protocol | Requests | Successes | p95 ms | Origin requests | Reused | Budget |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| h1 | 5 | 5 | 1.71 | 2 | 3 | pass |
| h2 | 5 | 5 | 2.16 | 2 | 3 | pass |
| h3 | 5 | 5 | 1.79 | 2 | 3 | pass |

Release budgets enforced by `kubio-bench`:

| Protocol | p95 budget |
| --- | ---: |
| h1 | 100 ms |
| h2 | 150 ms |
| h3 | 300 ms |

Release workflow uploads h1, h2, and h3 benchmark JSON artifacts under
`dist/bench/`.
