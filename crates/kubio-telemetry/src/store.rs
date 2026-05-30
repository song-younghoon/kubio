use kubio_store::StoreKind;

use crate::text::{metric, push_labels};

pub(crate) fn store_kind_label(kind: StoreKind) -> &'static str {
    match kind {
        StoreKind::Memory => "memory",
        StoreKind::Disk => "disk",
    }
}

pub(crate) fn store_operation_metrics(
    out: &mut String,
    store_kind: &str,
    operation: &str,
    count: u64,
    error_count: u64,
) {
    metric(
        out,
        "kubio_store_operations_total",
        &[
            ("store", store_kind),
            ("operation", operation),
            ("result", "ok"),
        ],
        count.saturating_sub(error_count),
    );
    metric(
        out,
        "kubio_store_operations_total",
        &[
            ("store", store_kind),
            ("operation", operation),
            ("result", "error"),
        ],
        error_count,
    );
}

pub(crate) fn store_operation_latency(
    out: &mut String,
    store_kind: &str,
    operation: &str,
    total_latency_us: u64,
) {
    out.push_str("kubio_store_operation_duration_seconds_sum");
    push_labels(out, &[("store", store_kind), ("operation", operation)]);
    out.push(' ');
    out.push_str(&format!("{:.6}", total_latency_us as f64 / 1_000_000.0));
    out.push('\n');
}
