use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use kubio_core::{DecisionReason, EffectiveConfig, Mode, ServerConfig};
use kubio_observe::{EventType, Observer};
use kubio_policy::PolicyEngine;
use kubio_proxy::{run_proxy, ProxyState};
use kubio_store::{CacheStore, MemoryStore};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use url::Url;

#[tokio::test]
async fn watch_mode_forwards_and_observes() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Watch, 100, 5, 20).await;

    let body = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "stable");

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.origin_requests, 1);
    assert_eq!(snapshot.overview.reused_responses, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn authorization_is_protected_and_never_stored() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;
    let client = reqwest::Client::new();

    for _ in 0..3 {
        let response = client
            .get(format!("{}/auth", runtime.proxy_url()))
            .header("authorization", "Bearer secret")
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.protected_requests, 3);
    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn no_store_response_is_not_cached() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 1, 1, 1).await;

    for _ in 0..3 {
        let response = reqwest::get(format!("{}/nostore", runtime.proxy_url()))
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    assert_eq!(runtime.store.stats().entries, 0);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn auto_mode_reuses_after_shadow_confidence() {
    let origin = TestOrigin::start().await;
    let runtime = TestRuntime::start(origin.url(), Mode::Auto, 2, 2, 1).await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/stable", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "stable");
    }

    let snapshot = runtime.observer.snapshot();
    assert_eq!(snapshot.overview.origin_requests, 2);
    assert_eq!(snapshot.overview.reused_responses, 1);
    assert_eq!(runtime.store.stats().entries, 1);
    runtime.shutdown().await;
    origin.shutdown().await;
}

#[tokio::test]
async fn panic_switch_stops_and_restores_reuse() {
    let origin = TestOrigin::start().await;
    let panic_file = temp_panic_file();
    let runtime =
        TestRuntime::start_with_panic(origin.url(), Mode::Auto, 2, 2, 1, Some(panic_file.clone()))
            .await;

    for _ in 0..3 {
        let body = reqwest::get(format!("{}/stable", runtime.proxy_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "stable");
    }
    assert_eq!(runtime.observer.snapshot().overview.reused_responses, 1);

    std::fs::write(&panic_file, "disabled").unwrap();
    for _ in 0..2 {
        let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
            .await
            .unwrap();
        assert!(response.status().is_success());
    }
    let active_snapshot = runtime.observer.snapshot();
    assert_eq!(active_snapshot.overview.reused_responses, 1);
    assert!(active_snapshot
        .routes
        .iter()
        .any(|route| route.reasons.contains(&DecisionReason::PanicSwitchActive)));
    assert_eq!(
        active_snapshot
            .events
            .iter()
            .filter(|event| event.event_type == EventType::PanicSwitchEnabled)
            .count(),
        1
    );

    std::fs::remove_file(&panic_file).unwrap();
    let response = reqwest::get(format!("{}/stable", runtime.proxy_url()))
        .await
        .unwrap();
    assert!(response.status().is_success());
    let restored_snapshot = runtime.observer.snapshot();
    assert_eq!(restored_snapshot.overview.reused_responses, 2);
    assert_eq!(
        restored_snapshot
            .events
            .iter()
            .filter(|event| event.event_type == EventType::PanicSwitchDisabled)
            .count(),
        1
    );

    runtime.shutdown().await;
    origin.shutdown().await;
}

struct TestOrigin {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestOrigin {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/stable", get(|| async { "stable" }))
            .route("/auth", get(origin_counted))
            .route("/nostore", get(no_store))
            .route("/echo", post(echo))
            .with_state(hits);
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Self {
            addr,
            shutdown: Some(tx),
        }
    }

    fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.addr)).unwrap()
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn origin_counted(State(hits): State<Arc<AtomicUsize>>) -> String {
    hits.fetch_add(1, Ordering::SeqCst);
    "auth".to_string()
}

async fn no_store() -> impl IntoResponse {
    ([("cache-control", "no-store")], "no-store")
}

async fn echo(request: Request<Body>) -> impl IntoResponse {
    request.into_body()
}

struct TestRuntime {
    addr: SocketAddr,
    observer: Arc<Observer>,
    store: Arc<MemoryStore>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestRuntime {
    async fn start(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
    ) -> Self {
        Self::start_with_panic(
            origin,
            mode,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
            None,
        )
        .await
    }

    async fn start_with_panic(
        origin: Url,
        mode: Mode,
        min_route_samples: u64,
        min_key_repeats: u64,
        min_shadow_validations: u64,
        panic_file: Option<PathBuf>,
    ) -> Self {
        let addr = unused_addr().await;
        let defaults = EffectiveConfig::default();
        let mut policy_config = defaults.policy.clone();
        policy_config.min_route_samples = min_route_samples;
        policy_config.min_key_repeats = min_key_repeats;
        policy_config.min_shadow_validations = min_shadow_validations;
        let config = EffectiveConfig {
            origin,
            mode,
            server: ServerConfig { listen: addr },
            policy: policy_config,
            panic_file,
            ..defaults
        };

        let config = Arc::new(config);
        let observer = Arc::new(Observer::new(
            100,
            100,
            100,
            min_route_samples,
            min_key_repeats,
            min_shadow_validations,
        ));
        let store = Arc::new(MemoryStore::new(&config.storage));
        let policy = Arc::new(PolicyEngine::new(&config));
        let state = ProxyState::new(config, policy, observer.clone(), store.clone()).unwrap();
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = run_proxy(state, async {
                let _ = rx.await;
            })
            .await;
        });

        let runtime = Self {
            addr,
            observer,
            store,
            shutdown: Some(tx),
        };
        runtime.wait_ready().await;
        runtime
    }

    fn proxy_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn wait_ready(&self) {
        for _ in 0..50 {
            if TcpStream::connect(self.addr).await.is_ok() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn unused_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn temp_panic_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "kubio-panic-switch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    path
}
