# Metrics

Metrics are exposed from the local dashboard server:

```text
GET /metrics
```

The path defaults to `/metrics` and can be changed with `observability.metrics_path`. Set `observability.metrics: false` to disable the endpoint.

Required metrics include:

- `kubio_requests_total`
- `kubio_origin_requests_total`
- `kubio_reused_responses_total`
- `kubio_protected_requests_total`
- `kubio_bypass_requests_total`
- `kubio_shadow_matches_total`
- `kubio_shadow_mismatches_total`
- `kubio_cache_entries`
- `kubio_cache_bytes`
- `kubio_cache_evictions_total`
- `kubio_request_duration_seconds`
- `kubio_origin_duration_seconds`
- `kubio_policy_decisions_total`

Allowed labels are bounded to method, route id, decision, status class, and quantile. Raw paths, query strings, user identifiers, header values, and IP addresses are not used as metric labels.
