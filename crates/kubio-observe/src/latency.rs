use kubio_core::{LatencyBucketSnapshot, LatencySnapshot};
use std::collections::VecDeque;
use std::time::Duration;

pub(crate) fn latency_snapshot(values: &VecDeque<Duration>) -> LatencySnapshot {
    if values.is_empty() {
        return LatencySnapshot::default();
    }
    let mut millis = values
        .iter()
        .map(|value| value.as_secs_f64() * 1000.0)
        .collect::<Vec<_>>();
    millis.sort_by(|left, right| left.total_cmp(right));
    let sum_ms = millis.iter().sum::<f64>();
    let avg_ms = sum_ms / millis.len() as f64;
    let buckets = latency_buckets(&millis);
    LatencySnapshot {
        p50_ms: percentile(&millis, 0.50),
        p95_ms: percentile(&millis, 0.95),
        avg_ms,
        count: millis.len() as u64,
        sum_ms,
        buckets,
    }
}

fn latency_buckets(sorted_millis: &[f64]) -> Vec<LatencyBucketSnapshot> {
    const BUCKETS_SECONDS: &[f64] = &[
        0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.000, 2.500, 5.000,
    ];

    BUCKETS_SECONDS
        .iter()
        .map(|le_seconds| LatencyBucketSnapshot {
            le_seconds: *le_seconds,
            count: sorted_millis
                .iter()
                .filter(|millis| **millis <= le_seconds * 1000.0)
                .count() as u64,
        })
        .collect()
}

pub(crate) fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[index.min(values.len() - 1)]
}
