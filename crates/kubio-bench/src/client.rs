use crate::args::BenchProtocol;
#[cfg(feature = "experimental-http3")]
use crate::h3::H3BenchClient;
use crate::proxy::ManagedProxy;
use anyhow::{Context, Result};

pub(crate) enum BenchClient {
    Http(reqwest::Client),
    #[cfg(feature = "experimental-http3")]
    H3(H3BenchClient),
}

impl BenchClient {
    pub(crate) async fn connect(protocol: BenchProtocol, proxy: &ManagedProxy) -> Result<Self> {
        #[cfg(not(feature = "experimental-http3"))]
        let _ = proxy;

        match protocol {
            BenchProtocol::H1 => Ok(Self::Http(reqwest::Client::new())),
            BenchProtocol::H2 => Ok(Self::Http(
                reqwest::Client::builder()
                    .http2_prior_knowledge()
                    .build()
                    .context("build h2 benchmark client")?,
            )),
            BenchProtocol::H3 => {
                #[cfg(feature = "experimental-http3")]
                {
                    Ok(Self::H3(
                        H3BenchClient::connect(proxy.http3_addr().context("missing h3 addr")?)
                            .await?,
                    ))
                }
                #[cfg(not(feature = "experimental-http3"))]
                {
                    anyhow::bail!("h3 benchmark requires --features experimental-http3");
                }
            }
        }
    }

    pub(crate) async fn get_path(
        &mut self,
        proxy: &ManagedProxy,
        path: &str,
        expected_prefix: &str,
    ) -> bool {
        match self {
            Self::Http(client) => match client
                .get(format!("{}{}", proxy.http_url(), path))
                .send()
                .await
                .and_then(|response| response.error_for_status())
            {
                Ok(response) => response
                    .text()
                    .await
                    .map(|body| body.starts_with(expected_prefix))
                    .unwrap_or(false),
                Err(_) => false,
            },
            #[cfg(feature = "experimental-http3")]
            Self::H3(client) => client
                .get(proxy.http3_addr().expect("h3 addr"), path)
                .await
                .map(|body| body.starts_with(expected_prefix))
                .unwrap_or(false),
        }
    }

    pub(crate) fn close(self) {
        match self {
            Self::Http(_) => {}
            #[cfg(feature = "experimental-http3")]
            Self::H3(client) => client.close(),
        }
    }
}
