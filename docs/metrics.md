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
- `kubio_revalidation_attempts_total`
- `kubio_revalidation_outcomes_total`
- `kubio_stale_responses_served_total`
- `kubio_stale_responses_denied_total`
- `kubio_route_hints_applied_total`
- `kubio_route_hints_rejected_total`
- `kubio_query_hints_applied_total`
- `kubio_query_hints_rejected_total`
- `kubio_query_param_suggestions_total`
- `kubio_store_errors_total`
- `kubio_request_duration_seconds`
- `kubio_origin_duration_seconds`
- `kubio_policy_decisions_total`

Latency metrics use Prometheus histogram samples:

```text
kubio_request_duration_seconds_bucket{route_id="GET /api/products",le="0.050"} 12
kubio_request_duration_seconds_sum{route_id="GET /api/products"} 0.42
kubio_request_duration_seconds_count{route_id="GET /api/products"} 20
```

Allowed labels are bounded to method, route id, decision, outcome, status class, store kind, and histogram bucket. Raw paths, query strings, user identifiers, header values, disk paths, and IP addresses are not used as metric labels.
