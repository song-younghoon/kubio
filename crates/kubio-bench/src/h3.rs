use anyhow::{bail, Result};
use bytes::{Buf, Bytes, BytesMut};
use http::Request;
use quinn::crypto::rustls::QuicClientConfig;
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) struct H3BenchClient {
    send: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    endpoint: quinn::Endpoint,
    driver: tokio::task::JoinHandle<()>,
}

impl H3BenchClient {
    pub(crate) async fn connect(addr: SocketAddr) -> Result<Self> {
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

    pub(crate) async fn get(&mut self, addr: SocketAddr, path: &str) -> Result<String> {
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

    pub(crate) fn close(self) {
        self.endpoint.close(0_u32.into(), b"done");
        self.driver.abort();
    }
}

pub(crate) fn unused_udp_addr() -> Result<SocketAddr> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let addr = socket.local_addr()?;
    drop(socket);
    Ok(addr)
}

pub(crate) fn tls_cert_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/localhost-cert.pem")
}

pub(crate) fn tls_key_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/localhost-key.pem")
}

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
