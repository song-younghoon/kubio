use anyhow::{bail, Context, Result};
use axum::routing::get;
use axum::Router;
#[cfg(feature = "experimental-http3")]
use bytes::{Buf, Bytes, BytesMut};
use clap::{Parser, ValueEnum};
#[cfg(feature = "experimental-http3")]
use http::Request;
#[cfg(feature = "experimental-http3")]
use kubio_core::TlsConfig;
use kubio_core::{EffectiveConfig, Mode};
use kubio_observe::{Observer, ProtocolCounts};
use kubio_policy::PolicyEngine;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::{CacheStore, MemoryStore};
#[cfg(feature = "experimental-http3")]
use quinn::crypto::rustls::QuicClientConfig;
use serde::Serialize;
#[cfg(feature = "experimental-http3")]
use std::fs::File;
#[cfg(feature = "experimental-http3")]
use std::io::BufReader;
use std::net::SocketAddr;
#[cfg(feature = "experimental-http3")]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use url::Url;

#[derive(Debug, Parser)]
#[command(name = "kubio-bench", about = "Local kubio protocol benchmark runner")]
struct Args {
    #[arg(long, value_enum, default_value_t = Scenario::Smoke)]
    scenario: Scenario,
    #[arg(long, value_enum, default_value_t = BenchProtocol::H1)]
    protocol: BenchProtocol,
    #[arg(long, default_value_t = 20)]
    requests: usize,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    output: OutputFormat,
    #[arg(long)]
    fail_on_budget: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
enum Scenario {
    Smoke,
    FreshHit,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BenchProtocol {
    H1,
    H2,
    H3,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Serialize)]
struct BenchReport {
    scenario: Scenario,
    protocol: BenchProtocol,
    requests: usize,
    successes: usize,
    failures: usize,
    p50_latency_ms: f64,
    p95_latency_ms: f64,
    observed_requests: u64,
    origin_requests: u64,
    reused_responses: u64,
    downstream_protocols: ProtocolCounts,
    upstream_protocols: ProtocolCounts,
    cache_entries: u64,
    budget: BudgetReport,
}

#[derive(Debug, Serialize)]
struct BudgetReport {
    passed: bool,
    min_success_rate: f64,
    max_p95_latency_ms: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let report = run(args.protocol, args.scenario, args.requests).await?;
    match args.output {
        OutputFormat::Text => print_text_report(&report),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    if args.fail_on_budget && !report.budget.passed {
        bail!("benchmark budget failed");
    }
    Ok(())
}

async fn run(protocol: BenchProtocol, scenario: Scenario, requests: usize) -> Result<BenchReport> {
    if protocol == BenchProtocol::H3 && !cfg!(feature = "experimental-http3") {
        bail!("h3 benchmark requires --features experimental-http3");
    }
    let origin = ManagedOrigin::start().await?;
    let proxy = ManagedProxy::start(origin.url(), protocol).await?;
    let mut latencies = Vec::with_capacity(requests);
    let mut successes = 0usize;

    #[cfg(feature = "experimental-http3")]
    let mut h3_client = if protocol == BenchProtocol::H3 {
        Some(H3BenchClient::connect(proxy.http3_addr.context("missing h3 addr")?).await?)
    } else {
        None
    };

    let client = match protocol {
        BenchProtocol::H1 | BenchProtocol::H3 => reqwest::Client::new(),
        BenchProtocol::H2 => reqwest::Client::builder()
            .http2_prior_knowledge()
            .build()
            .context("build h2 benchmark client")?,
    };

    for _ in 0..requests {
        let started = Instant::now();
        let ok = match protocol {
            BenchProtocol::H1 | BenchProtocol::H2 => client
                .get(format!("{}/stable", proxy.http_url()))
                .send()
                .await
                .and_then(|response| response.error_for_status())
                .is_ok(),
            BenchProtocol::H3 => {
                #[cfg(feature = "experimental-http3")]
                {
                    h3_client
                        .as_mut()
                        .expect("h3 client")
                        .get(proxy.http3_addr.expect("h3 addr"), "/stable")
                        .await
                        .map(|body| body == "stable")
                        .unwrap_or(false)
                }
                #[cfg(not(feature = "experimental-http3"))]
                false
            }
        };
        latencies.push(started.elapsed());
        if ok {
            successes += 1;
        }
    }

    #[cfg(feature = "experimental-http3")]
    if let Some(client) = h3_client {
        client.close();
    }

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

fn print_text_report(report: &BenchReport) {
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

fn budget_p95_ms(protocol: BenchProtocol, _scenario: Scenario) -> f64 {
    match protocol {
        BenchProtocol::H1 => 100.0,
        BenchProtocol::H2 => 150.0,
        BenchProtocol::H3 => 300.0,
    }
}

fn percentile_ms(values: &[Duration], percentile: f64) -> f64 {
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

struct ManagedOrigin {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ManagedOrigin {
    async fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new().route(
            "/stable",
            get(|| async { ([("cache-control", "public, max-age=60")], "stable") }),
        );
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Ok(Self {
            addr,
            shutdown: Some(tx),
        })
    }

    fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.addr)).expect("local origin URL")
    }
}

impl Drop for ManagedOrigin {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

struct ManagedProxy {
    addr: SocketAddr,
    #[cfg(feature = "experimental-http3")]
    http3_addr: Option<SocketAddr>,
    observer: Arc<Observer>,
    store: Arc<MemoryStore>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ManagedProxy {
    async fn start(origin: Url, protocol: BenchProtocol) -> Result<Self> {
        let addr = unused_addr().await?;
        let defaults = EffectiveConfig::default();
        let mut server = defaults.server.clone();
        server.listen = addr;
        match protocol {
            BenchProtocol::H1 => {}
            BenchProtocol::H2 => {
                server.protocols.http2 = true;
                server.protocols.h2c = true;
            }
            BenchProtocol::H3 => {
                #[cfg(feature = "experimental-http3")]
                {
                    server.tls = Some(TlsConfig {
                        cert: tls_cert_path(),
                        key: tls_key_path(),
                    });
                    server.http3.enabled = true;
                    server.http3.listen = Some(unused_udp_addr()?);
                }
            }
        }
        let mut policy = defaults.policy.clone();
        policy.min_route_samples = 2;
        policy.min_key_repeats = 2;
        policy.min_shadow_validations = 1;
        let config = Arc::new(EffectiveConfig {
            origin,
            mode: Mode::Auto,
            server,
            policy,
            ..defaults
        });
        let observer = Arc::new(Observer::new(100, 100, 100, 2, 2, 1));
        let store = Arc::new(MemoryStore::new(&config.storage));
        let policy = Arc::new(PolicyEngine::new(&config));
        let state = ProxyState::new(config.clone(), policy, observer.clone(), store.clone())?;
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = run_proxy(state, async {
                let _ = rx.await;
            })
            .await;
        });
        wait_tcp_ready(addr).await?;
        Ok(Self {
            addr,
            #[cfg(feature = "experimental-http3")]
            http3_addr: config.server.http3.listen,
            observer,
            store,
            shutdown: Some(tx),
        })
    }

    fn http_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for ManagedProxy {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn wait_tcp_ready(addr: SocketAddr) -> Result<()> {
    for _ in 0..50 {
        if TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    bail!("proxy did not become ready at {addr}")
}

async fn unused_addr() -> Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr)
}

#[cfg(feature = "experimental-http3")]
fn unused_udp_addr() -> Result<SocketAddr> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let addr = socket.local_addr()?;
    drop(socket);
    Ok(addr)
}

#[cfg(feature = "experimental-http3")]
fn tls_cert_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/localhost-cert.pem")
}

#[cfg(feature = "experimental-http3")]
fn tls_key_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/localhost-key.pem")
}

#[cfg(feature = "experimental-http3")]
struct H3BenchClient {
    send: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    endpoint: quinn::Endpoint,
    driver: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "experimental-http3")]
impl H3BenchClient {
    async fn connect(addr: SocketAddr) -> Result<Self> {
        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
        endpoint.set_default_client_config(h3_quinn_client_config()?);
        let connection = endpoint.connect(addr, "localhost")?.await?;
        let quic = h3_quinn::Connection::new(connection);
        let (mut connection, send) = h3::client::builder().build(quic).await?;
        let driver = tokio::spawn(async move {
            let _ = connection.wait_idle().await;
        });
        Ok(Self {
            send,
            endpoint,
            driver,
        })
    }

    async fn get(&mut self, addr: SocketAddr, path: &str) -> Result<String> {
        let uri = format!("https://localhost:{}{path}", addr.port());
        let mut stream = self.send.send_request(Request::get(uri).body(())?).await?;
        stream.finish().await?;
        let response = stream.recv_response().await?;
        if !response.status().is_success() {
            bail!("h3 response status {}", response.status());
        }
        let mut body = BytesMut::new();
        while let Some(mut chunk) = stream.recv_data().await? {
            let len = chunk.remaining();
            body.extend_from_slice(&chunk.copy_to_bytes(len));
        }
        Ok(String::from_utf8(body.to_vec())?)
    }

    fn close(self) {
        self.endpoint.close(0_u32.into(), b"done");
        self.driver.abort();
    }
}

#[cfg(feature = "experimental-http3")]
fn h3_quinn_client_config() -> Result<quinn::ClientConfig> {
    let mut roots = quinn::rustls::RootCertStore::empty();
    let file = File::open(tls_cert_path())?;
    for cert in rustls_pemfile::certs(&mut BufReader::new(file)) {
        roots.add(cert?)?;
    }
    let mut tls = quinn::rustls::ClientConfig::builder_with_provider(Arc::new(
        quinn::rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&quinn::rustls::version::TLS13])?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    Ok(quinn::ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(tls)?,
    )))
}
