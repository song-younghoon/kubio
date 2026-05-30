use anyhow::Result;
use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
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
        let app = Router::new()
            .route(
                "/stable",
                get(|| async { ([("cache-control", "public, max-age=60")], "stable") }),
            )
            .route("/notice/{id}", get(public_notice))
            .route("/catalog/{id}", get(public_catalog))
            .route("/user/{id}", get(public_user));
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

impl Drop for ManagedOrigin {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}
