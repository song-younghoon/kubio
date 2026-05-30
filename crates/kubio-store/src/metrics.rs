use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::store::StoreKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreStats {
    pub entries: u64,
    pub bytes: u64,
    pub evictions: u64,
    pub max_size: u64,
    pub max_object_size: u64,
    pub kind: StoreKind,
    pub disk_path: Option<String>,
    pub startup_recovered_entries: Option<u64>,
    pub corrupt_entries_skipped: Option<u64>,
    pub operations: StoreOperationMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreOperation {
    Get,
    Put,
    Purge,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreOperationMetrics {
    pub get: StoreOperationStats,
    pub put: StoreOperationStats,
    pub purge: StoreOperationStats,
    pub saturation_events: u64,
}

impl StoreOperationMetrics {
    pub(crate) fn record(
        &mut self,
        operation: StoreOperation,
        latency: Duration,
        success: bool,
        saturated: bool,
    ) {
        let stats = match operation {
            StoreOperation::Get => &mut self.get,
            StoreOperation::Put => &mut self.put,
            StoreOperation::Purge => &mut self.purge,
        };
        stats.record(latency, success);
        if saturated {
            self.saturation_events += 1;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreOperationStats {
    pub count: u64,
    pub error_count: u64,
    pub total_latency_us: u64,
}

impl StoreOperationStats {
    fn record(&mut self, latency: Duration, success: bool) {
        self.count += 1;
        if !success {
            self.error_count += 1;
        }
        self.total_latency_us = self
            .total_latency_us
            .saturating_add(latency.as_micros().min(u128::from(u64::MAX)) as u64);
    }
}
