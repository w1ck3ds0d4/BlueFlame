//! DNS-over-HTTPS resolver for the proxy's upstream connections.
//!
//! Default hyper-util `HttpConnector` resolves hostnames via tokio's
//! `lookup_host`, which on every platform delegates to the system stub
//! resolver - i.e. unencrypted UDP to whatever DNS the OS is configured
//! to use (typically the ISP's). For a privacy-first browser that's the
//! biggest unencrypted leak in the request path: even though TLS hides
//! the URL and the request body, the destination *hostname* is broadcast
//! in plaintext on every page load.
//!
//! This module wraps `hickory-resolver` (configured for DoH) in a tower
//! `Service<Name>` adapter that `HttpConnector::new_with_resolver`
//! accepts. The proxy swaps it in for the Direct upstream arm; SOCKS5
//! and built-in Tor arms already resolve remotely through the tunnel,
//! so DoH is a no-op there (and we don't want to double-resolve).
//!
//! Provider choice is exposed through Settings. The user can pick from a
//! built-in list (Cloudflare, Quad9, Google). "Off" keeps the system
//! resolver - the previous behavior - so users who explicitly want their
//! corporate or router-side DoH aren't overridden by us.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use hickory_resolver::config::{ResolverConfig, CLOUDFLARE, GOOGLE, QUAD9};
use hickory_resolver::{Resolver, TokioResolver};
use hyper_util::client::legacy::connect::dns::Name;
use tower_service::Service;

/// Built-in DoH endpoints. The string IDs are stable and live in
/// SQLite; treat them like enum discriminants on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DohProvider {
    /// System stub resolver. No DoH.
    #[default]
    Off,
    /// Cloudflare's `1.1.1.1` resolver.
    Cloudflare,
    /// Quad9's `9.9.9.9` malware-blocking resolver.
    Quad9,
    /// Google's `8.8.8.8`.
    Google,
}

impl DohProvider {
    pub fn id(self) -> &'static str {
        match self {
            DohProvider::Off => "off",
            DohProvider::Cloudflare => "cloudflare",
            DohProvider::Quad9 => "quad9",
            DohProvider::Google => "google",
        }
    }

    /// Human-readable label for the Footprint view and the Settings
    /// dropdown.
    pub fn display_name(self) -> &'static str {
        match self {
            DohProvider::Off => "system DNS (no DoH)",
            DohProvider::Cloudflare => "Cloudflare (1.1.1.1)",
            DohProvider::Quad9 => "Quad9 (9.9.9.9)",
            DohProvider::Google => "Google (8.8.8.8)",
        }
    }

    /// DoH endpoint URL queried under the hood. Returned mainly for the
    /// Footprint view so the user can see where their lookups actually
    /// go. `None` means "system resolver".
    pub fn doh_url(self) -> Option<&'static str> {
        match self {
            DohProvider::Off => None,
            DohProvider::Cloudflare => Some("https://cloudflare-dns.com/dns-query"),
            DohProvider::Quad9 => Some("https://dns.quad9.net/dns-query"),
            DohProvider::Google => Some("https://dns.google/dns-query"),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "off" => Some(DohProvider::Off),
            "cloudflare" => Some(DohProvider::Cloudflare),
            "quad9" => Some(DohProvider::Quad9),
            "google" => Some(DohProvider::Google),
            _ => None,
        }
    }

    pub fn all() -> &'static [DohProvider] {
        &[
            DohProvider::Off,
            DohProvider::Cloudflare,
            DohProvider::Quad9,
            DohProvider::Google,
        ]
    }
}

/// Tower-compatible DoH resolver. Cheap to clone (everything inside is
/// already arc'd); built once at proxy boot and used for every dial in
/// the Direct upstream arm.
#[derive(Clone)]
pub struct DohResolver {
    inner: Arc<TokioResolver>,
}

impl DohResolver {
    /// Build a resolver pinned to the given DoH provider. Returns None
    /// for `Off` - callers should keep the default tokio resolver in
    /// that case to avoid forcing system DNS through us unnecessarily.
    /// Errors propagate as Err so the caller can fall back to system
    /// DNS rather than silently skipping DoH on a misconfigured build.
    pub fn build(provider: DohProvider) -> Result<Option<Self>, String> {
        let group = match provider {
            DohProvider::Cloudflare => &CLOUDFLARE,
            DohProvider::Quad9 => &QUAD9,
            DohProvider::Google => &GOOGLE,
            DohProvider::Off => return Ok(None),
        };
        let config = ResolverConfig::https(group);
        // `TokioResolver::builder_with_config` uses the default tokio
        // runtime provider, which is what hickory's docs recommend
        // wherever we already have a tokio runtime up - which we do
        // (proxy::start is on tokio).
        let builder = Resolver::builder_with_config(config, Default::default());
        let resolver = builder
            .build()
            .map_err(|e| format!("doh resolver build: {e}"))?;
        Ok(Some(Self {
            inner: Arc::new(resolver),
        }))
    }
}

/// What hyper-util's `HttpConnector` calls when it needs to resolve a
/// hostname. Returns an iterator of `SocketAddr` (port left at 0; hyper
/// fills it in from the URI).
impl Service<Name> for DohResolver {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let resolver = self.inner.clone();
        Box::pin(async move {
            let host = name.as_str();
            let lookup = resolver
                .lookup_ip(host)
                .await
                .map_err(|e| std::io::Error::other(format!("doh lookup {host}: {e}")))?;
            let addrs: Vec<SocketAddr> = lookup.iter().map(|ip| SocketAddr::new(ip, 0)).collect();
            if addrs.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("doh {host}: no records"),
                ));
            }
            Ok(addrs.into_iter())
        })
    }
}
