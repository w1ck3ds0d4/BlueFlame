//! A `tower::Service<Uri>` connector that tunnels TCP connections through a
//! SOCKS5 proxy. Wrapped by `hyper_rustls::HttpsConnector` so HTTPS works.
//!
//! Used by the MITM proxy's upstream client when the user has toggled
//! "route upstream through SOCKS5" in settings - typically pointing at a
//! local Tor daemon on `127.0.0.1:9050` or Tor Browser's `127.0.0.1:9150`.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::Uri;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;
use tower_service::Service;

#[derive(Debug, Clone)]
pub struct Socks5Connector {
    proxy: SocketAddr,
}

impl Socks5Connector {
    pub fn new(proxy: SocketAddr) -> Self {
        Self { proxy }
    }
}

/// Newtype wrapper that gives the SOCKS5 stream a `Connection` impl so
/// `hyper_util`'s legacy client will accept it as a real connection handle.
pub struct SocksIo(pub Socks5Stream<TcpStream>);

impl AsyncRead for SocksIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for SocksIo {
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

impl Connection for SocksIo {
    fn connected(&self) -> Connected {
        // We don't have ALPN info or special proxy hints to set; the defaults
        // are fine for plain TCP tunneled through SOCKS5.
        Connected::new()
    }
}

impl Service<Uri> for Socks5Connector {
    type Response = TokioIo<SocksIo>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let proxy = self.proxy;
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

            let raw = TcpStream::connect(proxy).await?;
            let socks = Socks5Stream::connect_with_socket(raw, (host.as_str(), port))
                .await
                .map_err(io::Error::other)?;
            Ok(TokioIo::new(SocksIo(socks)))
        })
    }
}
