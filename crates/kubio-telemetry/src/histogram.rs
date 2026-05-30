use kubio_core::LatencySnapshot;

use crate::text::{metric, push_labels};

pub(crate) fn histogram(out: &mut String, name: &str, route_id: &str, latency: &LatencySnapshot) {
    let bucket_name = format!("{name}_bucket");
    for bucket in &latency.buckets {
        metric(
            out,
            &bucket_name,
            &[
                ("route_id", route_id),
                ("le", &format!("{:.3}", bucket.le_seconds)),
            ],
            bucket.count,
        );
    }
    metric(
        out,
        &bucket_name,
        &[("route_id", route_id), ("le", "+Inf")],
        latency.count,
    );
    out.push_str(name);
    out.push_str("_sum");
    push_labels(out, &[("route_id", route_id)]);
    out.push(' ');
    out.push_str(&format!("{:.6}", latency.sum_ms / 1000.0));
    out.push('\n');
    metric(
        out,
        &format!("{name}_count"),
        &[("route_id", route_id)],
        latency.count,
    );
}
