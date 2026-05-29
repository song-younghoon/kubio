use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use kubio_core::{
    parse_size, DashboardConfig, EffectiveConfig, FreshnessProfile, Mode, ObservabilityConfig,
    PolicyConfig, RouteId, ServerConfig, StorageConfig,
};
use kubio_dashboard::{run_dashboard, DashboardState};
use kubio_observe::{Observer, RouteSnapshot};
use kubio_policy::PolicyEngine;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::{MemoryStore, PurgeResult, PurgeSelector};
use kubio_telemetry::init_tracing;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};
use url::Url;

#[derive(Debug, Parser)]
#[command(name = "kubio", version, about = "Safe API response reuse autopilot")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ServeArgs),
    Routes(AdminArgs),
    Explain(ExplainArgs),
    Doctor(DoctorArgs),
    Purge(PurgeArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[arg(long = "to")]
    origin: Option<String>,
    #[arg(long, help = "proxy listen address; default: 0.0.0.0:8080")]
    listen: Option<SocketAddr>,
    #[arg(long, help = "dashboard listen address; default: 127.0.0.1:9900")]
    dashboard: Option<SocketAddr>,
    #[arg(long, help = "runtime mode: watch, shadow, or auto; default: watch")]
    mode: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(
        long,
        help = "freshness profile: strict, balanced, relaxed; default: balanced"
    )]
    freshness: Option<String>,
    #[arg(long)]
    debug_headers: bool,
    #[arg(long)]
    panic_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct AdminArgs {
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    dashboard: String,
}

#[derive(Debug, Args)]
struct ExplainArgs {
    route: String,
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    dashboard: String,
}

#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    to: Option<String>,
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    dashboard: String,
}

#[derive(Debug, Args)]
struct PurgeArgs {
    #[arg(long)]
    all: bool,
    #[arg(long)]
    route: Option<String>,
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    dashboard: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => serve(args).await,
        Command::Routes(args) => routes(args).await,
        Command::Explain(args) => explain(args).await,
        Command::Doctor(args) => doctor(args).await,
        Command::Purge(args) => purge(args).await,
    }
}

async fn serve(args: ServeArgs) -> Result<()> {
    let config = Arc::new(load_config_for_serve(&args)?);
    validate_config(&config)?;

    let observer = Arc::new(Observer::new(
        config.observability.max_routes,
        config.observability.max_keys,
        config.observability.max_events,
        config.policy.min_route_samples,
        config.policy.min_key_repeats,
        config.policy.min_shadow_validations,
    ));
    let store = Arc::new(MemoryStore::new(&config.storage));
    let policy = Arc::new(PolicyEngine::new(&config));

    print_startup(&config);

    let (shutdown_tx, _) = broadcast::channel::<()>(4);
    let proxy_state = ProxyState::new(config.clone(), policy, observer.clone(), store.clone())?;
    let mut proxy_shutdown = shutdown_tx.subscribe();
    let proxy_task = tokio::spawn(async move {
        run_proxy(proxy_state, async move {
            let _ = proxy_shutdown.recv().await;
        })
        .await
    });

    let dashboard_task = if config.dashboard.enabled {
        let dashboard_state = DashboardState {
            config: config.clone(),
            observer: observer.clone(),
            store: store.clone(),
        };
        let mut dashboard_shutdown = shutdown_tx.subscribe();
        Some(tokio::spawn(async move {
            run_dashboard(dashboard_state, async move {
                let _ = dashboard_shutdown.recv().await;
            })
            .await
        }))
    } else {
        None
    };

    tokio::select! {
        result = proxy_task => {
            result.context("proxy task join failed")??;
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown signal received");
            let _ = shutdown_tx.send(());
        }
    }

    if let Some(task) = dashboard_task {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => warn!(error = %err, "dashboard stopped with error"),
            Err(err) => warn!(error = %err, "dashboard task join failed"),
        }
    }
    Ok(())
}

async fn routes(args: AdminArgs) -> Result<()> {
    let url = format!("{}/api/routes", args.dashboard.trim_end_matches('/'));
    let snapshot: kubio_observe::ObserverSnapshot = reqwest::get(url).await?.json().await?;
    if snapshot.routes.is_empty() {
        println!("No routes observed yet.");
        return Ok(());
    }
    for route in snapshot.routes {
        println!(
            "{}\t{}\trequests={}\torigin={}\treused={}\tprotected={}",
            route.route_id.as_label(),
            route.state,
            route.request_count,
            route.origin_count,
            route.reuse_count,
            route.protected_count
        );
    }
    Ok(())
}

async fn explain(args: ExplainArgs) -> Result<()> {
    let route_id = parse_route_id(&args.route).context("route must look like `GET /path`")?;
    let url = format!(
        "{}/api/routes/by-hash/{}",
        args.dashboard.trim_end_matches('/'),
        route_id.hash()
    );
    let response = reqwest::get(url).await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("route has not been observed: {}", route_id.as_label());
    }
    let route: RouteSnapshot = response.json().await?;
    println!("{}\nStatus: {}\n", route.route_id.as_label(), route.state);
    println!("kubio's reasoning:");
    if route.explanation.is_empty() {
        println!("- kubio has not recorded enough information yet.");
    } else {
        for line in route.explanation {
            println!("- {line}");
        }
    }
    println!(
        "\nRequests: {}\nOrigin: {}\nReused: {}\nShadow matches: {}\nShadow mismatches: {}",
        route.request_count,
        route.origin_count,
        route.reuse_count,
        route.shadow_matches,
        route.shadow_mismatches
    );
    Ok(())
}

async fn doctor(args: DoctorArgs) -> Result<()> {
    let mut checks = Vec::new();
    let config_result = if let Some(path) = args.config.as_ref() {
        load_config_file(path).map(|_| ())
    } else {
        Ok(())
    };
    checks.push(("config parsing", config_result.is_ok()));

    if let Some(origin) = args.to.as_ref() {
        let origin_ok = Url::parse(origin).is_ok()
            && reqwest::get(origin)
                .await
                .map(|response| {
                    response.status().is_success() || response.status().is_redirection()
                })
                .unwrap_or(false);
        checks.push(("origin connectivity", origin_ok));
    }

    let dashboard = args.dashboard.trim_end_matches('/');
    let dashboard_ok = reqwest::get(format!("{dashboard}/api/overview"))
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false);
    checks.push(("dashboard", dashboard_ok));

    let metrics_ok = reqwest::get(format!("{dashboard}/metrics"))
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false);
    checks.push(("metrics endpoint", metrics_ok));

    for (name, ok) in &checks {
        println!("{name}: {}", if *ok { "ok" } else { "failed" });
    }

    if checks.iter().all(|(_, ok)| *ok) {
        Ok(())
    } else {
        bail!("doctor found failing checks")
    }
}

async fn purge(args: PurgeArgs) -> Result<()> {
    let selector = if args.all {
        PurgeRequest {
            selector: "all".to_string(),
            route_id: None,
            cache_key_hash: None,
        }
    } else if let Some(route) = args.route {
        PurgeRequest {
            selector: "route".to_string(),
            route_id: Some(route),
            cache_key_hash: None,
        }
    } else {
        bail!("provide --all or --route");
    };
    let client = reqwest::Client::new();
    let result: PurgeResult = client
        .post(format!(
            "{}/api/purge",
            args.dashboard.trim_end_matches('/')
        ))
        .json(&selector)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "Purged {} entries ({} bytes).",
        result.purged_entries, result.purged_bytes
    );
    Ok(())
}

fn print_startup(config: &EffectiveConfig) {
    println!("kubio is watching your API.\n");
    println!("Origin: {}", config.origin);
    println!("Proxy:  http://{}", config.server.listen);
    println!("Mode:   {}", title_case(&config.mode.to_string()));
    match config.mode {
        Mode::Watch => {
            println!("\nResponse reuse is not active yet.");
            println!("kubio will learn which responses are safe to reuse.");
        }
        Mode::Shadow => {
            println!("\nResponse reuse is not active yet.");
            println!("kubio will validate whether repeated responses are stable.");
        }
        Mode::Auto => {
            println!("\nResponse reuse is active for verified safe responses.");
        }
    }
    if config.dashboard.enabled {
        println!("\nDashboard: http://{}", config.dashboard.listen);
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn load_config_for_serve(args: &ServeArgs) -> Result<EffectiveConfig> {
    let file = if let Some(path) = args.config.as_ref() {
        Some(load_config_file(path)?)
    } else {
        None
    };

    let origin_set = args.origin.is_some()
        || file
            .as_ref()
            .and_then(|config| config.origin.as_ref())
            .is_some();
    if !origin_set {
        bail!("origin URL is required; pass --to <URL> or set origin in config");
    }

    let mut config = EffectiveConfig::default();
    if let Some(file) = file {
        apply_file_config(&mut config, file)?;
    }

    if let Some(origin) = args.origin.as_ref() {
        config.origin = Url::parse(origin).context("parse --to origin URL")?;
    }
    if let Some(listen) = args.listen {
        config.server.listen = listen;
    }
    if let Some(dashboard) = args.dashboard {
        config.dashboard.listen = dashboard;
    }
    if let Some(mode) = args.mode.as_ref() {
        config.mode = mode.parse().map_err(anyhow::Error::msg)?;
    }
    if let Some(freshness) = args.freshness.as_ref() {
        config.freshness = freshness.parse().map_err(anyhow::Error::msg)?;
    }
    if args.debug_headers {
        config.debug_headers = true;
    }
    if args.panic_file.is_some() {
        config.panic_file = args.panic_file.clone();
    }

    Ok(config)
}

fn load_config_file(path: &PathBuf) -> Result<FileConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read config file {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("parse config file {}", path.display()))
}

fn apply_file_config(config: &mut EffectiveConfig, file: FileConfig) -> Result<()> {
    if let Some(origin) = file.origin {
        config.origin = Url::parse(&origin).context("parse config origin")?;
    }
    if let Some(mode) = file.mode {
        config.mode = mode.parse().map_err(anyhow::Error::msg)?;
    }
    if let Some(freshness) = file.freshness {
        config.freshness = freshness.parse().map_err(anyhow::Error::msg)?;
    }
    if let Some(server) = file.server {
        if let Some(listen) = server.listen {
            config.server = ServerConfig {
                listen: listen.parse().context("parse server.listen")?,
            };
        }
    }
    if let Some(dashboard) = file.dashboard {
        if let Some(enabled) = dashboard.enabled {
            config.dashboard.enabled = enabled;
        }
        if let Some(listen) = dashboard.listen {
            config.dashboard.listen = listen.parse().context("parse dashboard.listen")?;
        }
        if let Some(allow_public) = dashboard.allow_public {
            config.dashboard.allow_public = allow_public;
        }
        if let Some(admin_api) = dashboard.admin_api {
            config.dashboard.admin_api = admin_api;
        }
    }
    if let Some(policy) = file.policy {
        apply_policy_config(&mut config.policy, policy)?;
    }
    if let Some(storage) = file.storage {
        if let Some(kind) = storage.kind {
            config.storage.kind = kind;
        }
        if let Some(max_size) = storage.max_size {
            config.storage.max_size = parse_size(&max_size).map_err(anyhow::Error::msg)?;
        }
        if let Some(max_object_size) = storage.max_object_size {
            config.storage.max_object_size =
                parse_size(&max_object_size).map_err(anyhow::Error::msg)?;
            config.policy.max_object_size = config.storage.max_object_size;
        }
    }
    if let Some(observability) = file.observability {
        if let Some(metrics) = observability.metrics {
            config.observability.metrics = metrics;
        }
        if let Some(metrics_path) = observability.metrics_path {
            config.observability.metrics_path = metrics_path;
        }
        if let Some(tracing) = observability.tracing {
            config.observability.tracing = tracing;
        }
    }
    if let Some(debug_headers) = file.debug_headers {
        config.debug_headers = debug_headers;
    }
    if let Some(panic_file) = file.panic_file {
        config.panic_file = Some(PathBuf::from(panic_file));
    }
    if let Some(admin_token) = file.admin_token {
        config.admin_token = Some(admin_token);
    }
    Ok(())
}

fn apply_policy_config(config: &mut PolicyConfig, policy: FilePolicyConfig) -> Result<()> {
    if let Some(value) = policy.respect_origin_headers {
        config.respect_origin_headers = value;
    }
    if let Some(value) = policy.protect_authorization {
        config.protect_authorization = value;
    }
    if let Some(value) = policy.protect_cookies {
        config.protect_cookies = value;
    }
    if let Some(value) = policy.protect_set_cookie {
        config.protect_set_cookie = value;
    }
    if let Some(value) = policy.max_object_size {
        config.max_object_size = parse_size(&value).map_err(anyhow::Error::msg)?;
    }
    if let Some(value) = policy.max_fingerprint_body_size {
        config.max_fingerprint_body_size = parse_size(&value).map_err(anyhow::Error::msg)?;
    }
    if let Some(value) = policy.min_route_samples {
        config.min_route_samples = value;
    }
    if let Some(value) = policy.min_key_repeats {
        config.min_key_repeats = value;
    }
    if let Some(value) = policy.min_shadow_validations {
        config.min_shadow_validations = value;
    }
    if let Some(value) = policy.max_shadow_mismatch_rate {
        config.max_shadow_mismatch_rate = value;
    }
    Ok(())
}

fn validate_config(config: &EffectiveConfig) -> Result<()> {
    if config.origin.scheme() != "http" && config.origin.scheme() != "https" {
        bail!("origin must use http or https");
    }
    if config.storage.kind != "memory" {
        bail!("v0.1.0 only supports memory storage");
    }
    let dashboard_ip = config.dashboard.listen.ip();
    let public_dashboard = !matches!(dashboard_ip, IpAddr::V4(ip) if ip.is_loopback())
        && !matches!(dashboard_ip, IpAddr::V6(ip) if ip.is_loopback());
    if public_dashboard && !config.dashboard.allow_public {
        bail!("public dashboard binding requires dashboard.allow_public: true");
    }
    if public_dashboard && config.dashboard.admin_api && config.admin_token.is_none() {
        bail!("public dashboard admin API requires admin_token");
    }
    Ok(())
}

fn parse_route_id(value: &str) -> Option<RouteId> {
    let (method, path) = value.split_once(' ')?;
    Some(RouteId::new(method, path))
}

#[derive(Debug, Clone, Deserialize)]
struct FileConfig {
    #[allow(dead_code)]
    version: Option<u64>,
    server: Option<FileServerConfig>,
    origin: Option<String>,
    mode: Option<String>,
    freshness: Option<String>,
    dashboard: Option<FileDashboardConfig>,
    policy: Option<FilePolicyConfig>,
    storage: Option<FileStorageConfig>,
    observability: Option<FileObservabilityConfig>,
    debug_headers: Option<bool>,
    panic_file: Option<String>,
    admin_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileServerConfig {
    listen: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileDashboardConfig {
    enabled: Option<bool>,
    listen: Option<String>,
    allow_public: Option<bool>,
    admin_api: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct FilePolicyConfig {
    respect_origin_headers: Option<bool>,
    protect_authorization: Option<bool>,
    protect_cookies: Option<bool>,
    protect_set_cookie: Option<bool>,
    max_object_size: Option<String>,
    max_fingerprint_body_size: Option<String>,
    min_route_samples: Option<u64>,
    min_key_repeats: Option<u64>,
    min_shadow_validations: Option<u64>,
    max_shadow_mismatch_rate: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileStorageConfig {
    kind: Option<String>,
    max_size: Option<String>,
    max_object_size: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileObservabilityConfig {
    metrics: Option<bool>,
    metrics_path: Option<String>,
    tracing: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct PurgeRequest {
    selector: String,
    route_id: Option<String>,
    cache_key_hash: Option<String>,
}

#[allow(dead_code)]
fn _default_config_parts() -> (DashboardConfig, StorageConfig, ObservabilityConfig) {
    (
        EffectiveConfig::default().dashboard,
        EffectiveConfig::default().storage,
        EffectiveConfig::default().observability,
    )
}

#[allow(dead_code)]
fn _selector_examples(route: RouteId) -> (PurgeSelector, FreshnessProfile) {
    (PurgeSelector::Route(route), FreshnessProfile::Balanced)
}
