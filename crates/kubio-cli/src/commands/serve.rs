use crate::args::ServeArgs;
use crate::config::{load_config_for_serve, validate_config};
use anyhow::{Context, Result};
use kubio_core::{EffectiveConfig, Mode};
use kubio_dashboard::{run_dashboard, DashboardState};
use kubio_observe::Observer;
use kubio_policy::PolicyEngine;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::{CacheStore, DiskStore, MemoryStore};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

pub(crate) async fn serve(args: ServeArgs) -> Result<()> {
    let no_update_check = args.no_update_check;
    let config = Arc::new(load_config_for_serve(&args)?);
    validate_config(&config)?;

    let observer = Arc::new(Observer::with_adaptive_config(
        config.observability.max_routes,
        config.observability.max_keys,
        config.observability.max_events,
        config.policy.min_route_samples,
        config.policy.min_key_repeats,
        config.policy.min_shadow_validations,
        config.policy.adaptive_reuse.clone(),
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

    crate::commands::spawn_ambient_update_check(no_update_check);

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

fn print_startup(config: &EffectiveConfig) {
    println!("kubio is watching your API.\n");
    println!("Origin: {}", config.origin);
    println!(
        "Proxy:  {}://{}",
        if config.server.tls.is_some() {
            "https"
        } else {
            "http"
        },
        config.server.listen
    );
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
    println!(
        "Protocols: http1={} http2={} h2c={} http3={}",
        config.server.protocols.http1,
        config.server.protocols.http2,
        config.server.protocols.h2c,
        config.server.http3.enabled
    );
    println!(
        "Origin protocol: {} (fallback={})",
        config.origin_protocol.preferred, config.origin_protocol.fallback
    );
    println!(
        "HTTP/3 build support: {}",
        cfg!(feature = "experimental-http3")
    );
    if config.origin_protocol.http3_experimental {
        println!(
            "Origin HTTP/3: experimental=true idle_pool={} idle_timeout={}s",
            config.origin_protocol.http3_max_idle_connections,
            config.origin_protocol.http3_idle_timeout.as_secs()
        );
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
