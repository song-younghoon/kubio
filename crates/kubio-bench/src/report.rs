use crate::args::{BenchProtocol, Scenario};
use kubio_observe::ProtocolCounts;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub(crate) struct BenchReport {
    pub(crate) scenario: Scenario,
    pub(crate) protocol: BenchProtocol,
    pub(crate) requests: usize,
    pub(crate) successes: usize,
    pub(crate) failures: usize,
    pub(crate) p50_latency_ms: f64,
    pub(crate) p95_latency_ms: f64,
    pub(crate) observed_requests: u64,
    pub(crate) origin_requests: u64,
    pub(crate) reused_responses: u64,
    pub(crate) downstream_protocols: ProtocolCounts,
    pub(crate) upstream_protocols: ProtocolCounts,
    pub(crate) cache_entries: u64,
    pub(crate) budget: BudgetReport,
}

#[derive(Debug, Serialize)]
pub(crate) struct BudgetReport {
    pub(crate) passed: bool,
    pub(crate) min_success_rate: f64,
    pub(crate) max_p95_latency_ms: f64,
}

pub(crate) fn print_text_report(report: &BenchReport) {
    println!(
        "scenario={:?} protocol={:?} requests={} successes={} failures={} p50_ms={:.2} p95_ms={:.2} reused={} budget={}",
        report.scenario,
        report.protocol,
        report.requests,
        report.successes,
        report.failures,
        report.p50_latency_ms,
        report.p95_latency_ms,
        report.reused_responses,
        if report.budget.passed { "pass" } else { "fail" }
    );
}

pub(crate) fn budget_p95_ms(protocol: BenchProtocol, _scenario: Scenario) -> f64 {
    match protocol {
        BenchProtocol::H1 => 100.0,
        BenchProtocol::H2 => 150.0,
        BenchProtocol::H3 => 300.0,
    }
}

pub(crate) fn percentile_ms(values: &[Duration], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut millis = values
        .iter()
        .map(|value| value.as_secs_f64() * 1000.0)
        .collect::<Vec<_>>();
    millis.sort_by(|left, right| left.total_cmp(right));
    let index = ((millis.len() - 1) as f64 * percentile).round() as usize;
    millis[index.min(millis.len() - 1)]
}
