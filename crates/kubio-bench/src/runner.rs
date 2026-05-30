use crate::args::{BenchProtocol, Scenario};
use crate::client::BenchClient;
use crate::origin::ManagedOrigin;
use crate::proxy::ManagedProxy;
use crate::report::{budget_p95_ms, percentile_ms, BenchReport, BudgetReport};
use anyhow::{bail, Result};
use kubio_observe::ProtocolCounts;
use kubio_store::CacheStore;
use std::time::Instant;

pub(crate) async fn run(
    protocol: BenchProtocol,
    scenario: Scenario,
    requests: usize,
) -> Result<BenchReport> {
    if protocol == BenchProtocol::H3 && !cfg!(feature = "experimental-http3") {
        bail!("h3 benchmark requires --features experimental-http3");
    }
    let origin = ManagedOrigin::start().await?;
    let proxy = ManagedProxy::start(origin.url(), protocol).await?;
    let mut client = BenchClient::connect(protocol, &proxy).await?;
    let mut latencies = Vec::with_capacity(requests);
    let mut successes = 0usize;

    for _ in 0..requests {
        let started = Instant::now();
        let ok = client.get_stable(&proxy).await;
        latencies.push(started.elapsed());
        if ok {
            successes += 1;
        }
    }

    client.close();

    let snapshot = proxy.observer.snapshot();
    let stats = proxy.store.stats();
    let p50_latency_ms = percentile_ms(&latencies, 0.50);
    let p95_latency_ms = percentile_ms(&latencies, 0.95);
    let success_rate = if requests == 0 {
        0.0
    } else {
        successes as f64 / requests as f64
    };
    let budget = BudgetReport {
        passed: success_rate >= 1.0 && p95_latency_ms <= budget_p95_ms(protocol, scenario),
        min_success_rate: 1.0,
        max_p95_latency_ms: budget_p95_ms(protocol, scenario),
    };

    Ok(BenchReport {
        scenario,
        protocol,
        requests,
        successes,
        failures: requests.saturating_sub(successes),
        p50_latency_ms,
        p95_latency_ms,
        observed_requests: snapshot.overview.observed_requests,
        origin_requests: snapshot.overview.origin_requests,
        reused_responses: snapshot.overview.reused_responses,
        downstream_protocols: ProtocolCounts {
            http1: snapshot.overview.downstream_http1_requests,
            http2: snapshot.overview.downstream_http2_requests,
            http3: snapshot.overview.downstream_http3_requests,
        },
        upstream_protocols: ProtocolCounts {
            http1: snapshot.overview.upstream_http1_requests,
            http2: snapshot.overview.upstream_http2_requests,
            http3: snapshot.overview.upstream_http3_requests,
        },
        cache_entries: stats.entries,
        budget,
    })
}
