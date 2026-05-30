use crate::args::{AdminArgs, DoctorArgs, ExplainArgs, PurgeArgs};
use crate::config::{apply_file_config, load_config_file, validate_config};
use anyhow::{bail, Context, Result};
use kubio_core::{EffectiveConfig, OriginProtocolPreference, RouteId};
use kubio_observe::{ProtocolCounts, RouteSnapshot};
use kubio_store::PurgeResult;
use serde::Serialize;
use std::path::PathBuf;
use url::Url;

pub(crate) async fn routes(args: AdminArgs) -> Result<()> {
    let url = format!("{}/api/routes", args.dashboard.trim_end_matches('/'));
    let snapshot: kubio_observe::ObserverSnapshot = reqwest::get(url).await?.json().await?;
    if snapshot.routes.is_empty() {
        println!("No routes observed yet.");
        return Ok(());
    }
    for route in snapshot.routes {
        println!(
            "{}\t{}\trequests={}\torigin={}\treused={}\tprotected={}\trevalidated={}\tstale={}\tdownstream={}\tupstream={}",
            route.route_id.as_label(),
            route.state,
            route.request_count,
            route.origin_count,
            route.reuse_count,
            route.protected_count,
            route.revalidation_attempts,
            route.stale_served,
            protocol_counts_label(&route.downstream_protocols),
            protocol_counts_label(&route.upstream_protocols)
        );
    }
    Ok(())
}

pub(crate) async fn explain(args: ExplainArgs) -> Result<()> {
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
        "\nRequests: {}\nOrigin: {}\nReused: {}\nShadow matches: {}\nShadow mismatches: {}\nRevalidations: {}\nStale served: {}\nDownstream protocols: {}\nUpstream protocols: {}",
        route.request_count,
        route.origin_count,
        route.reuse_count,
        route.shadow_matches,
        route.shadow_mismatches,
        route.revalidation_attempts,
        route.stale_served,
        protocol_counts_label(&route.downstream_protocols),
        protocol_counts_label(&route.upstream_protocols)
    );
    Ok(())
}

pub(crate) async fn doctor(args: DoctorArgs) -> Result<()> {
    let no_update_check = args.no_update_check;
    let mut checks = Vec::new();

    let (file_config, effective_config, config_ok) = if let Some(path) = args.config.as_ref() {
        match load_config_file(path) {
            Ok(file_config) => {
                let mut effective = EffectiveConfig::default();
                let ok = apply_file_config(&mut effective, file_config.clone())
                    .and_then(|_| validate_config(&effective))
                    .is_ok();
                (Some(file_config), Some(effective), ok)
            }
            Err(_) => (None, None, false),
        }
    } else {
        (None, Some(EffectiveConfig::default()), true)
    };
    checks.push(("config parsing", config_ok));
    if let Some(config) = effective_config.as_ref() {
        checks.push((
            "http/2 config",
            !config.server.protocols.http2
                || config.server.tls.is_some()
                || config.server.protocols.h2c,
        ));
        checks.push((
            "http/3 build support",
            (!config.server.http3.enabled && !config.origin_protocol.http3_experimental)
                || cfg!(feature = "experimental-http3"),
        ));
        checks.push((
            "http/3 runtime config",
            (!config.server.http3.enabled || config.server.tls.is_some())
                && (!config.origin_protocol.http3_experimental
                    || config.origin_protocol.preferred == OriginProtocolPreference::Http3
                    || config.origin_protocol.preferred == OriginProtocolPreference::Auto),
        ));
    }

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

    let _ = crate::commands::run_ambient_update_check(no_update_check).await;

    if checks.iter().all(|(_, ok)| *ok) {
        Ok(())
    } else {
        bail!("doctor found failing checks")
    }
}

pub(crate) async fn purge(args: PurgeArgs) -> Result<()> {
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

fn protocol_counts_label(counts: &ProtocolCounts) -> String {
    format!(
        "http1:{},http2:{},http3:{}",
        counts.http1, counts.http2, counts.http3
    )
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

#[derive(Debug, Clone, Serialize)]
struct PurgeRequest {
    selector: String,
    route_id: Option<String>,
    cache_key_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
