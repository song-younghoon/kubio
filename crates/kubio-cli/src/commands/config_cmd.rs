use crate::args::{
    ConfigArgs, ConfigCheckArgs, ConfigDiffArgs, ConfigReloadArgs, ConfigStatusArgs,
    ConfigSubcommand,
};
use crate::config::{apply_file_config, load_config_file, validate_config};
use anyhow::{Context, Result};
use kubio_core::{
    ConfigChangeClass, ConfigCheckRequest, ConfigReloadRequest, ConfigReloadResult,
    ConfigReloadSnapshot, EffectiveConfig, ReloadStatus,
};

pub(crate) async fn config(args: ConfigArgs) -> Result<()> {
    match args.command {
        ConfigSubcommand::Check(args) => check(args).await,
        ConfigSubcommand::Reload(args) => reload(args).await,
        ConfigSubcommand::Diff(args) => diff(args).await,
        ConfigSubcommand::Status(args) => status(args).await,
    }
}

async fn check(args: ConfigCheckArgs) -> Result<()> {
    let file = load_config_file(&args.config)?;
    if file.origin.is_none() {
        anyhow::bail!("origin URL is required in config");
    }
    let mut config = EffectiveConfig::default();
    apply_file_config(&mut config, file)?;
    validate_config(&config)?;
    println!("config ok: {}", args.config.display());
    Ok(())
}

async fn reload(args: ConfigReloadArgs) -> Result<()> {
    let client = reqwest::Client::new();
    let request = client
        .post(dashboard_url(
            args.dashboard.trim_end_matches('/'),
            "/api/config/reload",
        ))
        .json(&ConfigReloadRequest {
            dry_run: args.dry_run,
        });
    let result: ConfigReloadResult = with_admin_token(request, args.admin_token.as_deref())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    print_reload_result(&result);
    if matches!(
        result.status,
        ReloadStatus::Applied | ReloadStatus::DryRunOk
    ) {
        Ok(())
    } else {
        anyhow::bail!("config reload failed: {}", result.status)
    }
}

async fn diff(args: ConfigDiffArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.config)
        .with_context(|| format!("read config file {}", args.config.display()))?;
    let client = reqwest::Client::new();
    let request = client
        .post(dashboard_url(
            args.dashboard.trim_end_matches('/'),
            "/api/config/check",
        ))
        .json(&ConfigCheckRequest { config: Some(text) });
    let result: ConfigReloadResult = with_admin_token(request, args.admin_token.as_deref())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    print_reload_result(&result);
    Ok(())
}

async fn status(args: ConfigStatusArgs) -> Result<()> {
    let url = dashboard_url(
        args.dashboard.trim_end_matches('/'),
        "/api/config/reload-status",
    );
    let status: ConfigReloadSnapshot = reqwest::get(url).await?.error_for_status()?.json().await?;
    println!("active_generation={}", status.active_generation);
    println!(
        "config_source={}",
        status.config_source.as_deref().unwrap_or("none")
    );
    println!(
        "last_reload={}",
        status
            .last_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!("reloadable_changes={}", status.last_reloadable_change_count);
    println!("restart_required={}", status.last_restart_required_count);
    println!("routes_demoted={}", status.last_routes_demoted);
    println!("cache_entries_purged={}", status.last_cache_entries_purged);
    Ok(())
}

fn print_reload_result(result: &ConfigReloadResult) {
    println!("status={}", result.status);
    println!("active_generation={}", result.active_generation);
    println!("message={}", result.message);
    if let Some(previous) = result.previous_generation {
        println!("previous_generation={previous}");
    }
    if !result.diff.is_empty() {
        println!("reloadable:");
        for entry in result
            .diff
            .iter()
            .filter(|entry| entry.class == ConfigChangeClass::Reloadable)
        {
            println!("  {}: {}", entry.path, entry.summary);
        }
        println!("restart_required:");
        for entry in result
            .diff
            .iter()
            .filter(|entry| entry.class == ConfigChangeClass::RestartRequired)
        {
            println!("  {}: {}", entry.path, entry.summary);
        }
    }
    if result.routes_added > 0 || result.routes_changed > 0 || result.routes_removed > 0 {
        println!(
            "routes=added:{} changed:{} removed:{} demoted:{}",
            result.routes_added,
            result.routes_changed,
            result.routes_removed,
            result.routes_demoted
        );
    }
    if result.cache_entries_purged > 0 {
        println!("cache_entries_purged={}", result.cache_entries_purged);
    }
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
