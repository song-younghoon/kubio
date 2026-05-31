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
- `kubio_routes_by_reuse_class`
- `kubio_origin_public_fast_path_total`
- `kubio_precision_confidence_routes`
- `kubio_precision_canary_total`
- `kubio_query_equivalence_candidates_total`
- `kubio_response_header_equivalence_candidates_total`
- `kubio_response_header_ignored_total`
- `kubio_response_header_suppressed_on_hit_total`
- `kubio_variant_groups`
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
- `kubio_config_generation`
- `kubio_config_reload_attempts_total`
- `kubio_config_reload_changes_total`
- `kubio_config_reload_routes_total`
- `kubio_config_reload_cache_entries_purged_total`
- `kubio_request_duration_seconds`
- `kubio_origin_duration_seconds`
- `kubio_policy_decisions_total`

Latency metrics use Prometheus histogram samples:

```text
kubio_request_duration_seconds_bucket{route_id="GET /api/products",le="0.050"} 12
kubio_request_duration_seconds_sum{route_id="GET /api/products"} 0.42
kubio_request_duration_seconds_count{route_id="GET /api/products"} 20
```

Allowed labels are bounded to method, route id, decision, outcome, status class,
store kind, reload status, reload change class, reload route action, protocol,
and histogram bucket. Raw paths, query strings, user identifiers, header values,
disk paths, config paths, and IP addresses are not used as metric labels.

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

v0.5.3 reload metrics use bounded labels only:

```text
kubio_config_generation 2
kubio_config_reload_attempts_total{status="applied"} 1
kubio_config_reload_changes_total{class="reloadable"} 3
kubio_config_reload_changes_total{class="restart_required"} 0
kubio_config_reload_routes_total{action="demoted"} 1
kubio_config_reload_cache_entries_purged_total 2
```
