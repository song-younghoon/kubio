use anyhow::Result;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Response, StatusCode};
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use url::Url;

pub(crate) struct ManagedOrigin {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ManagedOrigin {
    pub(crate) async fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/stable",
                get(|| async { ([("cache-control", "public, max-age=60")], "stable") }),
            )
            .route("/notice/{id}", get(public_notice))
            .route("/catalog/{id}", get(public_catalog))
            .route("/user/{id}", get(public_user))
            .route("/query-intel", get(|| async { "query-intel" }))
            .route("/dynamic-response-id/{id}", get(dynamic_response_id))
            .route("/vendor-header/{id}", get(vendor_header))
            .route("/articles/{slug}", get(public_article))
            .route("/users/{slug}", get(public_user_slug))
            .route("/canary/1", get(canary_changing))
            .with_state(hits);
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

    pub(crate) fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.addr)).expect("local origin URL")
    }
}

async fn public_notice(Path(id): Path<String>) -> impl axum::response::IntoResponse {
    (
        [("cache-control", "public, max-age=60")],
        format!("notice-{id}"),
    )
}

async fn public_catalog(Path(id): Path<String>) -> String {
    format!("catalog-{id}")
}

async fn public_user(Path(id): Path<String>) -> impl axum::response::IntoResponse {
    (
        [("cache-control", "public, max-age=60")],
        format!("user-{id}"),
    )
}

async fn dynamic_response_id(
    Path(id): Path<String>,
    State(hits): State<Arc<AtomicUsize>>,
) -> impl axum::response::IntoResponse {
    let hit = hits.fetch_add(1, Ordering::SeqCst);
    Response::builder()
        .status(StatusCode::OK)
        .header("cache-control", "public, max-age=60")
        .header("content-type", "text/plain")
        .header("x-response-id", format!("res-{hit}"))
        .header("x-correlation-id", format!("corr-{hit}"))
        .body(Body::from(format!("dynamic-response-id-{id}")))
        .expect("valid response")
}

async fn vendor_header(
    Path(id): Path<String>,
    State(hits): State<Arc<AtomicUsize>>,
) -> impl axum::response::IntoResponse {
    let hit = hits.fetch_add(1, Ordering::SeqCst);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain")
        .header("x-vendor-execution-id", format!("exec-{hit}"))
        .body(Body::from(format!("vendor-header-{id}")))
        .expect("valid response")
}

async fn public_article(Path(slug): Path<String>) -> String {
    format!("article-{slug}")
}

async fn public_user_slug(Path(slug): Path<String>) -> impl axum::response::IntoResponse {
    (
        [("cache-control", "public, max-age=60")],
        format!("user-{slug}"),
    )
}

async fn canary_changing(
    State(hits): State<Arc<AtomicUsize>>,
) -> impl axum::response::IntoResponse {
    let hit = hits.fetch_add(1, Ordering::SeqCst);
    (
        [("cache-control", "public, max-age=60")],
        format!("canary-{hit}"),
    )
}

impl Drop for ManagedOrigin {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}
