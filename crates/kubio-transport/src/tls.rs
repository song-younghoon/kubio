use anyhow::Context;
use kubio_core::{EffectiveConfig, TlsConfig};
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;
use tokio_rustls::TlsAcceptor;

pub(crate) fn tls_acceptor(
    tls: &TlsConfig,
    config: &EffectiveConfig,
) -> anyhow::Result<TlsAcceptor> {
    let mut server = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(load_tls_certificates(tls)?, load_tls_private_key(tls)?)
        .context("build TLS server config")?;
    server.alpn_protocols = alpn_protocols(config);
    Ok(TlsAcceptor::from(Arc::new(server)))
}

pub(crate) fn load_tls_certificates(
    tls: &TlsConfig,
) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let certs = CertificateDer::pem_file_iter(&tls.cert)
        .with_context(|| format!("open TLS cert {}", tls.cert.display()))?
        .collect::<Result<Vec<_>, _>>()
        .context("read TLS certificates")?;
    if certs.is_empty() {
        anyhow::bail!(
            "TLS cert file {} contained no certificates",
            tls.cert.display()
        );
    }
    Ok(certs)
}

pub(crate) fn load_tls_private_key(tls: &TlsConfig) -> anyhow::Result<PrivateKeyDer<'static>> {
    PrivateKeyDer::from_pem_file(&tls.key)
        .with_context(|| format!("read TLS private key {}", tls.key.display()))
}

pub(crate) fn alpn_protocols(config: &EffectiveConfig) -> Vec<Vec<u8>> {
    let mut protocols = Vec::new();
    if config.server.protocols.http2 {
        protocols.push(b"h2".to_vec());
    }
    if config.server.protocols.http1 {
        protocols.push(b"http/1.1".to_vec());
    }
    protocols
}
