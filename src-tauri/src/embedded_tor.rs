//! Embedded Tor client built on `arti-client` so users don't need an external
//! tor daemon. Bootstrapped once at proxy startup; the resulting `TorClient`
//! is shared into our `TorBuiltInConnector` which uses it to dial every
//! upstream target the MITM proxy needs.
//!
//! This whole module is gated on the `built-in-tor` Cargo feature via the
//! `mod` declaration in `lib.rs`.

use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use arti_client::{TorClient, TorClientConfig};
use hyper::Uri;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tor_rtcompat::PreferredRuntime;
use tower_service::Service;

pub type SharedTor = Arc<TorClient<PreferredRuntime>>;

/// Boot arti pointing at a per-app state/cache dir. This blocks on the
/// initial consensus fetch, which can take 5-30 seconds on first run; the
/// caller should spawn it on a task and treat the proxy as unavailable
/// until the returned future resolves.
pub async fn bootstrap(data_dir: &Path) -> anyhow::Result<SharedTor> {
    let tor_dir = data_dir.join("tor");
    let state_dir = tor_dir.join("state");
    let cache_dir = tor_dir.join("cache");
    std::fs::create_dir_all(&state_dir)?;
    std::fs::create_dir_all(&cache_dir)?;

    let mut builder = TorClientConfig::builder();
    builder
        .storage()
        .state_dir(arti_client::config::CfgPath::new_literal(&state_dir))
        .cache_dir(arti_client::config::CfgPath::new_literal(&cache_dir));
    let cfg = builder.build()?;

    tracing::info!("arti: bootstrapping (this can take up to ~30s on first run)");
    let client = TorClient::create_bootstrapped(cfg).await?;
    tracing::info!("arti: bootstrapped");
    Ok(Arc::new(client))
}

/// `tower::Service<Uri>` that dials its target through the embedded
/// `TorClient`. Mirrors `Socks5Connector` so `proxy::start` can plug either
/// upstream choice into the same `hyper_rustls` wrapper.
#[derive(Clone)]
pub struct TorBuiltInConnector {
    tor: SharedTor,
}

impl TorBuiltInConnector {
    pub fn new(tor: SharedTor) -> Self {
        Self { tor }
    }
}

/// Newtype so we can implement `hyper_util::Connection` on arti's stream.
pub struct ArtiIo(pub arti_client::DataStream);

impl AsyncRead for ArtiIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for ArtiIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl Connection for ArtiIo {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

impl Service<Uri> for TorBuiltInConnector {
    type Response = TokioIo<ArtiIo>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let tor = self.tor.clone();
        Box::pin(async move {
            let host = uri
                .host()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "uri has no host"))?
                .to_string();
            let port = uri.port_u16().unwrap_or_else(|| {
                if uri.scheme_str() == Some("https") {
                    443
                } else {
                    80
                }
            });
            let stream = tor
                .connect((host.as_str(), port))
                .await
                .map_err(io::Error::other)?;
            Ok(TokioIo::new(ArtiIo(stream)))
        })
    }
}
