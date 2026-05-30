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
- `kubio_observer_events_dropped_total`
- `kubio_store_operations_total`
- `kubio_store_operation_duration_seconds_sum`
- `kubio_store_saturation_events_total`
- `kubio_downstream_requests_total`
- `kubio_upstream_requests_total`
- `kubio_backpressure_rejections_total`
- `kubio_in_flight_requests`
- `kubio_max_in_flight_requests`
- `kubio_protocol_fallbacks_total`
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

v0.3.0 protocol labels are bounded enum values only:

```text
kubio_downstream_requests_total{protocol="http1"} 10
kubio_downstream_requests_total{protocol="http2"} 3
kubio_upstream_requests_total{protocol="http1"} 12
kubio_backpressure_rejections_total 1
kubio_in_flight_requests 0
kubio_protocol_fallbacks_total 0
kubio_store_operations_total{store="memory",operation="put",result="ok"} 2
kubio_store_operation_duration_seconds_sum{store="memory",operation="put"} 0.001200
kubio_store_saturation_events_total{store="memory"} 0
```
