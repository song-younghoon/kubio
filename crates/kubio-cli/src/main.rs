use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use kubio_core::{
    parse_duration, parse_size, DashboardConfig, EffectiveConfig, FreshnessProfile, Mode,
    ObservabilityConfig, PolicyConfig, RouteFreshnessConfig, RouteHintConfig, RouteId,
    RouteMatchConfig, RouteQueryConfig, RouteSafetyConfig, RouteStaleIfErrorConfig,
    RouteVaryConfig, StorageConfig,
};
use kubio_dashboard::{run_dashboard, DashboardState};
use kubio_observe::{Observer, RouteSnapshot};
use kubio_policy::PolicyEngine;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::{CacheStore, DiskStore, MemoryStore, PurgeResult, PurgeSelector};
use kubio_telemetry::init_tracing;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
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
    #[arg(long, env = "KUBIO_ADMIN_TOKEN")]
    admin_token: Option<String>,
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
    let store: Arc<dyn CacheStore> = match config.storage.kind.as_str() {
        "memory" => Arc::new(MemoryStore::new(&config.storage)),
        "disk" => Arc::new(DiskStore::open(&config.storage)?),
        _ => unreachable!("storage kind validated before startup"),
    };
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
        _ = shutdown_signal() => {
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
            "{}\t{}\trequests={}\torigin={}\treused={}\tprotected={}\trevalidated={}\tstale={}",
            route.route_id.as_label(),
            route.state,
            route.request_count,
            route.origin_count,
            route.reuse_count,
            route.protected_count,
            route.revalidation_attempts,
            route.stale_served
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
        "\nRequests: {}\nOrigin: {}\nReused: {}\nShadow matches: {}\nShadow mismatches: {}\nRevalidations: {}\nStale served: {}",
        route.request_count,
        route.origin_count,
        route.reuse_count,
        route.shadow_matches,
        route.shadow_mismatches,
        route.revalidation_attempts,
        route.stale_served
    );
    Ok(())
}

async fn doctor(args: DoctorArgs) -> Result<()> {
    let mut checks = Vec::new();

    let (file_config, config_ok) = if let Some(path) = args.config.as_ref() {
        match load_config_file(path) {
            Ok(file_config) => {
                let mut effective = EffectiveConfig::default();
                let ok = apply_file_config(&mut effective, file_config.clone())
                    .and_then(|_| validate_config(&effective))
                    .is_ok();
                (Some(file_config), ok)
            }
            Err(_) => (None, false),
        }
    } else {
        (None, true)
    };
    checks.push(("config parsing", config_ok));

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
    let overview_url = dashboard_url(dashboard, "/api/overview");
    let (dashboard_ok, storage_ok) = match reqwest::get(overview_url).await {
        Ok(response) if response.status().is_success() => {
            let storage_ok = response
                .json::<serde_json::Value>()
                .await
                .map(|value| {
                    value.get("cache_entries").is_some() && value.get("cache_bytes").is_some()
                })
                .unwrap_or(false);
            (true, storage_ok)
        }
        Ok(_) | Err(_) => (false, false),
    };
    checks.push(("dashboard", dashboard_ok));
    checks.push(("storage snapshot", storage_ok));

    let metrics_enabled = file_config
        .as_ref()
        .and_then(|config| config.observability.as_ref())
        .and_then(|observability| observability.metrics)
        .unwrap_or(true);
    if metrics_enabled {
        let metrics_path = file_config
            .as_ref()
            .and_then(|config| config.observability.as_ref())
            .and_then(|observability| observability.metrics_path.as_deref())
            .unwrap_or("/metrics");
        let metrics_ok = reqwest::get(dashboard_url(dashboard, metrics_path))
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false);
        checks.push(("metrics endpoint", metrics_ok));
    }
    let store_api_ok = reqwest::get(dashboard_url(dashboard, "/api/store"))
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false);
    checks.push(("store api", store_api_ok));

    if let Some(panic_file) = file_config
        .as_ref()
        .and_then(|config| config.panic_file.as_ref())
    {
        checks.push(("panic switch inactive", !PathBuf::from(panic_file).exists()));
    }

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
    let request = client
        .post(dashboard_url(
            args.dashboard.trim_end_matches('/'),
            "/api/purge",
        ))
        .json(&selector);
    let result: PurgeResult = with_admin_token(request, args.admin_token.as_deref())
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
    println!("Store: {}", config.storage.kind);
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            warn!(error = %err, "failed to install Ctrl-C shutdown handler");
        }
    };

    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(err) => {
                    warn!(error = %err, "failed to install SIGTERM shutdown handler");
                    ctrl_c.await;
                    return;
                }
            };

        tokio::select! {
            _ = ctrl_c => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    ctrl_c.await;
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
            config.server.listen = listen.parse().context("parse server.listen")?;
        }
        if let Some(origin_timeout_ms) = server.origin_timeout_ms {
            if origin_timeout_ms == 0 {
                bail!("server.origin_timeout_ms must be greater than zero");
            }
            config.server.origin_timeout = Duration::from_millis(origin_timeout_ms);
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
        if let Some(path) = storage.path {
            config.storage.path = Some(PathBuf::from(path));
        }
        if let Some(sync) = storage.sync {
            config.storage.sync = sync;
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
    if let Some(routes) = file.routes {
        config.routes = routes
            .into_iter()
            .map(RouteHintConfig::try_from)
            .collect::<Result<Vec<_>>>()?;
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
    if let Some(revalidation) = policy.revalidation {
        if let Some(enabled) = revalidation.enabled {
            config.revalidation.enabled = enabled;
        }
        if let Some(prefer_etag) = revalidation.prefer_etag {
            config.revalidation.prefer_etag = prefer_etag;
        }
        if let Some(max_validator_length) = revalidation.max_validator_length {
            config.revalidation.max_validator_length = max_validator_length;
        }
    }
    if let Some(stale_if_error) = policy.stale_if_error {
        if let Some(mode) = stale_if_error.mode {
            config.stale_if_error.mode = mode.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(max_stale) = stale_if_error.max_stale {
            config.stale_if_error.max_stale =
                parse_duration(&max_stale).map_err(anyhow::Error::msg)?;
        }
    }
    if let Some(query_intelligence) = policy.query_intelligence {
        if let Some(enabled) = query_intelligence.enabled {
            config.query_intelligence.enabled = enabled;
        }
        if let Some(auto_ignore) = query_intelligence.auto_ignore {
            config.query_intelligence.auto_ignore = auto_ignore;
        }
    }
    Ok(())
}

fn validate_config(config: &EffectiveConfig) -> Result<()> {
    if config.origin.scheme() != "http" && config.origin.scheme() != "https" {
        bail!("origin must use http or https");
    }
    if config.storage.kind != "memory" && config.storage.kind != "disk" {
        bail!("storage.kind must be memory or disk");
    }
    if config.server.origin_timeout.is_zero() {
        bail!("server.origin_timeout_ms must be greater than zero");
    }
    if config.storage.max_size == 0 {
        bail!("storage.max_size must be greater than zero");
    }
    if config.storage.max_object_size == 0 {
        bail!("storage.max_object_size must be greater than zero");
    }
    if config.storage.max_object_size > config.storage.max_size {
        bail!("storage.max_object_size must not exceed storage.max_size");
    }
    if config.policy.max_object_size == 0 {
        bail!("policy.max_object_size must be greater than zero");
    }
    if config.policy.max_fingerprint_body_size == 0 {
        bail!("policy.max_fingerprint_body_size must be greater than zero");
    }
    if config.policy.max_request_body_size == 0 {
        bail!("policy.max_request_body_size must be greater than zero");
    }
    if config.policy.min_route_samples == 0
        || config.policy.min_key_repeats == 0
        || config.policy.min_shadow_validations == 0
    {
        bail!("policy promotion thresholds must be greater than zero");
    }
    if !config.policy.max_shadow_mismatch_rate.is_finite()
        || !(0.0..=1.0).contains(&config.policy.max_shadow_mismatch_rate)
    {
        bail!("policy.max_shadow_mismatch_rate must be between 0 and 1");
    }
    if config.policy.revalidation.max_validator_length == 0 {
        bail!("policy.revalidation.max_validator_length must be greater than zero");
    }
    if config.policy.stale_if_error.max_stale.is_zero() {
        bail!("policy.stale_if_error.max_stale must be greater than zero");
    }
    validate_route_hints(&config.routes)?;
    if !valid_dashboard_path(&config.observability.metrics_path) {
        bail!("observability.metrics_path must be an absolute dashboard path");
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

fn valid_dashboard_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() > 1
        && !path.contains('{')
        && !path.contains('}')
        && !path.contains('*')
        && !path.chars().any(char::is_control)
}

fn validate_route_hints(routes: &[RouteHintConfig]) -> Result<()> {
    let mut seen_routes = Vec::new();
    for route in routes {
        if route.route_match.method.trim().is_empty() {
            bail!("routes[].match.method must not be empty");
        }
        if !route.route_match.path.starts_with('/') {
            bail!("routes[].match.path must be absolute");
        }
        let route_key = (
            route.route_match.method.to_ascii_uppercase(),
            route.route_match.path.clone(),
        );
        if seen_routes.contains(&route_key) {
            bail!(
                "duplicate route hint for {} {}",
                route.route_match.method,
                route.route_match.path
            );
        }
        seen_routes.push(route_key);
        for include in &route.query.include {
            if route
                .query
                .ignore
                .iter()
                .any(|ignore| query_patterns_overlap(include, ignore))
            {
                bail!("query parameter pattern `{include}` conflicts with the route ignore list");
            }
        }
        for pattern in route.query.ignore.iter().chain(route.query.include.iter()) {
            if pattern.is_empty() {
                bail!("query hint patterns must not be empty");
            }
            if pattern.matches('*').count() > 1
                || (pattern.contains('*') && !pattern.ends_with('*'))
            {
                bail!("query hint glob patterns only support a trailing *");
            }
        }
        if route
            .stale_if_error
            .max_stale
            .map(|duration| duration.is_zero())
            .unwrap_or(false)
        {
            bail!("routes[].stale_if_error.max_stale must be greater than zero");
        }
    }
    Ok(())
}

fn query_patterns_overlap(left: &str, right: &str) -> bool {
    match (left.strip_suffix('*'), right.strip_suffix('*')) {
        (Some(left_prefix), Some(right_prefix)) => {
            left_prefix.starts_with(right_prefix) || right_prefix.starts_with(left_prefix)
        }
        (Some(left_prefix), None) => right.starts_with(left_prefix),
        (None, Some(right_prefix)) => left.starts_with(right_prefix),
        (None, None) => left == right,
    }
}

fn parse_route_id(value: &str) -> Option<RouteId> {
    let (method, path) = value.split_once(' ')?;
    Some(RouteId::new(method, path))
}

fn dashboard_url(base: &str, path: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn with_admin_token(
    request: reqwest::RequestBuilder,
    admin_token: Option<&str>,
) -> reqwest::RequestBuilder {
    match admin_token {
        Some(token) => request.header("x-kubio-admin-token", token),
        None => request,
    }
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
    routes: Option<Vec<FileRouteHintConfig>>,
    debug_headers: Option<bool>,
    panic_file: Option<String>,
    admin_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileServerConfig {
    listen: Option<String>,
    origin_timeout_ms: Option<u64>,
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
    revalidation: Option<FileRevalidationConfig>,
    stale_if_error: Option<FileStaleIfErrorConfig>,
    query_intelligence: Option<FileQueryIntelligenceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileRevalidationConfig {
    enabled: Option<bool>,
    prefer_etag: Option<bool>,
    max_validator_length: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileStaleIfErrorConfig {
    mode: Option<String>,
    max_stale: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileQueryIntelligenceConfig {
    enabled: Option<bool>,
    auto_ignore: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileStorageConfig {
    kind: Option<String>,
    max_size: Option<String>,
    max_object_size: Option<String>,
    path: Option<String>,
    sync: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileObservabilityConfig {
    metrics: Option<bool>,
    metrics_path: Option<String>,
    tracing: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileRouteHintConfig {
    name: Option<String>,
    #[serde(rename = "match")]
    route_match: FileRouteMatchConfig,
    freshness: Option<FileRouteFreshnessConfig>,
    query: Option<RouteQueryConfig>,
    vary: Option<RouteVaryConfig>,
    stale_if_error: Option<FileRouteStaleIfErrorConfig>,
    safety: Option<RouteSafetyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileRouteMatchConfig {
    method: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FileRouteFreshnessConfig {
    ttl: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileRouteStaleIfErrorConfig {
    enabled: Option<bool>,
    max_stale: Option<String>,
}

impl TryFrom<FileRouteHintConfig> for RouteHintConfig {
    type Error = anyhow::Error;

    fn try_from(value: FileRouteHintConfig) -> Result<Self> {
        let query = match value.query {
            Some(query) if query.is_empty() => {
                bail!("routes[].query must define include or ignore patterns")
            }
            Some(query) => query,
            None => RouteQueryConfig::default(),
        };
        let freshness = RouteFreshnessConfig {
            ttl: value
                .freshness
                .and_then(|freshness| freshness.ttl)
                .map(|ttl| parse_duration(&ttl).map_err(anyhow::Error::msg))
                .transpose()?,
        };
        let stale_if_error = match value.stale_if_error {
            Some(stale) => RouteStaleIfErrorConfig {
                enabled: stale.enabled.unwrap_or(false),
                max_stale: stale
                    .max_stale
                    .map(|duration| parse_duration(&duration).map_err(anyhow::Error::msg))
                    .transpose()?,
            },
            None => RouteStaleIfErrorConfig::default(),
        };
        Ok(RouteHintConfig {
            name: value.name,
            route_match: RouteMatchConfig {
                method: value.route_match.method,
                path: value.route_match.path,
            },
            freshness,
            query,
            vary: value.vary.unwrap_or_default(),
            stale_if_error,
            safety: value.safety.unwrap_or_default(),
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_invalid_metrics_path() {
        let mut config = EffectiveConfig::default();
        config.observability.metrics_path = "metrics".to_string();

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("metrics_path"));
    }

    #[test]
    fn validation_rejects_zero_promotion_thresholds() {
        let mut config = EffectiveConfig::default();
        config.policy.min_shadow_validations = 0;

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("promotion thresholds"));
    }

    #[test]
    fn validation_rejects_duplicate_route_hints() {
        let config = EffectiveConfig {
            routes: vec![
                test_route_hint("GET", "/api/products"),
                test_route_hint("get", "/api/products"),
            ],
            ..EffectiveConfig::default()
        };

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("duplicate route hint"));
    }

    #[test]
    fn validation_rejects_query_glob_conflicts() {
        let hint = RouteHintConfig {
            query: RouteQueryConfig {
                include: vec!["utm_source".to_string()],
                ignore: vec!["utm_*".to_string()],
            },
            ..test_route_hint("GET", "/api/products")
        };
        let config = EffectiveConfig {
            routes: vec![hint],
            ..EffectiveConfig::default()
        };

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("conflicts"));
    }

    #[test]
    fn route_hint_config_rejects_empty_query_section() {
        let file: FileConfig = serde_yaml::from_str(
            r#"
origin: "http://localhost:3000"
routes:
  - match:
      method: GET
      path: "/api/products"
    query: {}
"#,
        )
        .unwrap();
        let mut config = EffectiveConfig::default();

        let err = apply_file_config(&mut config, file)
            .unwrap_err()
            .to_string();

        assert!(err.contains("routes[].query"));
    }

    #[test]
    fn dashboard_url_joins_base_and_path() {
        assert_eq!(
            dashboard_url("http://127.0.0.1:9900/", "metrics"),
            "http://127.0.0.1:9900/metrics"
        );
        assert_eq!(
            dashboard_url("http://127.0.0.1:9900", "/api/overview"),
            "http://127.0.0.1:9900/api/overview"
        );
    }

    fn test_route_hint(method: &str, path: &str) -> RouteHintConfig {
        RouteHintConfig {
            name: None,
            route_match: RouteMatchConfig {
                method: method.to_string(),
                path: path.to_string(),
            },
            freshness: RouteFreshnessConfig::default(),
            query: RouteQueryConfig::default(),
            vary: RouteVaryConfig::default(),
            stale_if_error: RouteStaleIfErrorConfig::default(),
            safety: RouteSafetyConfig::default(),
        }
    }
}
