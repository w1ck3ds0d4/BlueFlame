//! MITM proxy core.
//!
//! Listens on a local port, intercepts HTTP/HTTPS traffic from the Tauri
//! webview, applies filter rules, and proxies the allowed traffic upstream.
//!
//! Filter rules match URLs against regex patterns (compatible with easylist
//! basic entries). Matching requests return 204 No Content and increment the
//! blocked counter exposed to the UI.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use hudsucker::{
    certificate_authority::RcgenAuthority,
    hyper::{Request, Response, StatusCode},
    Body, HttpContext, HttpHandler, Proxy, RequestOrResponse,
};
use regex::RegexSet;
use tokio::sync::oneshot;

use crate::ca::RootCa;
use crate::reputation::ReputationStore;
use crate::security::SecurityStore;

#[derive(Debug, Default)]
pub struct ProxyStats {
    pub requests_total: AtomicU64,
    pub requests_blocked: AtomicU64,
    pub bytes_saved: AtomicU64,
}

impl ProxyStats {
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            requests_total: self.requests_total.load(Ordering::Relaxed),
            requests_blocked: self.requests_blocked.load(Ordering::Relaxed),
            bytes_saved: self.bytes_saved.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsSnapshot {
    pub requests_total: u64,
    pub requests_blocked: u64,
    pub bytes_saved: u64,
}

/// One entry in the recent-blocks ring buffer shown in the Dashboard.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BlockedEntry {
    /// Unix seconds when the block happened.
    pub ts: u64,
    /// The blocked URL.
    pub url: String,
}

/// How many recent block events to keep. Bounded so memory stays flat even
/// on a busy browsing session.
pub const BLOCK_LOG_CAPACITY: usize = 500;

#[derive(Debug, Default)]
pub struct BlockLog {
    inner: RwLock<VecDeque<BlockedEntry>>,
}

impl BlockLog {
    pub fn push(&self, url: String) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let entry = BlockedEntry { ts, url };
        let mut q = self.inner.write().expect("block log rwlock poisoned");
        if q.len() == BLOCK_LOG_CAPACITY {
            q.pop_front();
        }
        q.push_back(entry);
    }

    /// Return up to `limit` most-recent entries, newest first.
    pub fn recent(&self, limit: usize) -> Vec<BlockedEntry> {
        let q = self.inner.read().expect("block log rwlock poisoned");
        q.iter().rev().take(limit).cloned().collect()
    }

    pub fn clear(&self) {
        let mut q = self.inner.write().expect("block log rwlock poisoned");
        q.clear();
    }
}

pub struct ProxyState {
    /// Whether the proxy task is listening. Almost always `true` after boot -
    /// the proxy auto-starts at app init so the webview always has a target.
    pub running: bool,
    pub port: u16,
    /// Whether filter matching is active. Toggle this from the UI to
    /// pause filtering without tearing down the proxy (which would
    /// interrupt in-flight requests).
    pub filters_enabled: Arc<AtomicBool>,
    pub stats: Arc<ProxyStats>,
    /// Swappable at runtime - filter-list refreshes replace the inner `Arc`
    /// so the handler picks up new rules on its next request without restart.
    pub filters: Arc<RwLock<Arc<Vec<RegexSet>>>>,
    /// Recent blocked requests for the Dashboard's live log panel.
    pub block_log: Arc<BlockLog>,
    /// Per-host signal stash populated by the request handler. Feeds the
    /// trust scoring in `crate::trust::evaluate`.
    pub security: Arc<SecurityStore>,
    /// Host-level reputation feed (URLHaus etc.). Hot-swappable -
    /// boot-time hydration extends this without taking down the proxy.
    pub reputation: Arc<ReputationStore>,
    pub runner: Option<ProxyRunner>,
    /// Unix epoch seconds for when the proxy listener started. Used for
    /// the dashboard uptime readout.
    pub started_at: Option<u64>,
    /// Short label describing which upstream the currently-running proxy was
    /// booted with: `"direct"`, `"socks5:<addr>"`, or `"built-in-tor"`.
    /// Changing this at runtime requires restart.
    pub upstream_applied: String,
    /// Lifecycle of the embedded arti bootstrap so the UI can render a
    /// loading indicator. Empty string means "not applicable" (e.g. Tor
    /// isn't selected). Typical transitions when built-in Tor is on:
    /// `""` -> `"running"` -> (`"ready"` | `"failed:<msg>"`).
    pub tor_bootstrap: String,
}

impl Default for ProxyState {
    fn default() -> Self {
        Self {
            running: false,
            port: 0,
            filters_enabled: Arc::new(AtomicBool::new(true)),
            stats: Arc::new(ProxyStats::default()),
            filters: Arc::new(RwLock::new(Arc::new(vec![RegexSet::new(
                default_filter_patterns(),
            )
            .expect("built-in filter patterns must compile")]))),
            block_log: Arc::new(BlockLog::default()),
            security: Arc::new(SecurityStore::default()),
            reputation: Arc::new(ReputationStore::with_bundled()),
            runner: None,
            started_at: None,
            upstream_applied: "direct".to_string(),
            tor_bootstrap: String::new(),
        }
    }
}

/// Replace the active filter collection. Callers are the boot-time loader
/// and the manual refresh command. Multiple sets are OR'd on match.
pub fn swap_filters(state: &ProxyState, new: Vec<RegexSet>) {
    let mut guard = state.filters.write().expect("filters rwlock poisoned");
    *guard = Arc::new(new);
}

/// Whether the given host should always skip filter matching. The
/// always-allow list is derived from the configured search engines; we
/// don't want a contextual EasyList rule to block the user's own search.
/// Exact host match or subdomain match - so `duckduckgo.com` allows both
/// `duckduckgo.com` and `improving.duckduckgo.com` (the CDN).
fn host_is_always_allowed(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    crate::search::always_allowed_hosts()
        .iter()
        .any(|allowed| h == *allowed || h.ends_with(&format!(".{allowed}")))
}

/// Minimal built-in filter set. Always active on top of any loaded lists so
/// there is a floor of protection even if all network lists fail to fetch.
pub fn default_filter_patterns() -> Vec<&'static str> {
    vec![
        r"^https?://[^/]*doubleclick\.net/",
        r"^https?://[^/]*google-analytics\.com/",
        r"^https?://[^/]*googletagmanager\.com/",
        r"^https?://[^/]*facebook\.com/tr",
        r"^https?://[^/]*hotjar\.com/",
        r"^https?://[^/]*mixpanel\.com/",
        r"^https?://[^/]*segment\.(io|com)/",
        r"^https?://[^/]*amplitude\.com/",
    ]
}

#[derive(Clone)]
struct BlueFlameHandler {
    filters: Arc<RwLock<Arc<Vec<RegexSet>>>>,
    filters_enabled: Arc<AtomicBool>,
    stats: Arc<ProxyStats>,
    block_log: Arc<BlockLog>,
    security: Arc<SecurityStore>,
    /// URL of the request currently in flight through this handler
    /// instance. Set in `handle_request`, consumed in `handle_response`
    /// so the response-side signal collector knows which host to
    /// attribute the response headers to. hudsucker guarantees the
    /// request/response pair hits the same handler instance (see
    /// hudsucker::HttpHandler trait docs).
    pending_url: Option<String>,
}

impl HttpHandler for BlueFlameHandler {
    async fn handle_request(
        &mut self,
        _ctx: &HttpContext,
        req: Request<Body>,
    ) -> RequestOrResponse {
        self.stats.requests_total.fetch_add(1, Ordering::Relaxed);

        let url = req.uri().to_string();
        self.pending_url = Some(url.clone());
        let referer = req
            .headers()
            .get("referer")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        if self.filters_enabled.load(Ordering::Relaxed) {
            // Carve-out: hosts the user explicitly routes to (search engines)
            // must never be blocked. EasyList has contextual rules meant for
            // trackers that overblock these when applied unconditionally.
            let host_allowed = req
                .uri()
                .host()
                .map(host_is_always_allowed)
                .unwrap_or(false);
            let filters = {
                let guard = self.filters.read().expect("filters rwlock poisoned");
                guard.clone()
            };
            if !host_allowed && filters.iter().any(|s| s.is_match(&url)) {
                self.stats.requests_blocked.fetch_add(1, Ordering::Relaxed);
                // Rough estimate - actual bytes saved requires response inspection
                self.stats.bytes_saved.fetch_add(2048, Ordering::Relaxed);
                self.block_log.push(url.clone());
                self.security.record_request(&url, referer.as_deref(), true);
                // Blocked responses short-circuit here - clear the
                // pending slot so the next request on this handler
                // instance doesn't attribute its response to this URL.
                self.pending_url = None;
                tracing::debug!(%url, "blocked by filter");
                let res = Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .body(Body::from(&b""[..]))
                    .expect("static response body");
                return RequestOrResponse::Response(res);
            }
        }

        self.security
            .record_request(&url, referer.as_deref(), false);
        RequestOrResponse::Request(req)
    }

    async fn handle_response(&mut self, _ctx: &HttpContext, res: Response<Body>) -> Response<Body> {
        let Some(url) = self.pending_url.take() else {
            return res;
        };
        let headers = res.headers();
        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let has_csp = headers.contains_key("content-security-policy");
        let has_hsts = headers.contains_key("strict-transport-security");
        let has_xfo = headers.contains_key("x-frame-options");
        let has_xcto = headers.contains_key("x-content-type-options");
        let has_rp = headers.contains_key("referrer-policy");
        let has_pp =
            headers.contains_key("permissions-policy") || headers.contains_key("feature-policy");

        // Any Set-Cookie without Secure + HttpOnly is a weak vuln signal.
        // Session cookies that ride plain HTTP or are reachable from JS
        // are the classic theft vector.
        let insecure_cookie = headers.get_all("set-cookie").iter().any(|v| {
            let s = v.to_str().unwrap_or("").to_ascii_lowercase();
            !(s.contains("secure") && s.contains("httponly"))
        });

        self.security.record_response(
            &url,
            content_type.as_deref(),
            has_csp,
            has_hsts,
            has_xfo,
            has_xcto,
            has_rp,
            has_pp,
            insecure_cookie,
        );

        // Body analysis path. Gated so we don't buffer huge responses:
        //   - content-type must be text/html
        //   - content-length must be present and <= MAX_BODY_BYTES
        // When both hold, decode gzip/br, collect the bytes, parse as
        // HTML for login-form / password-on-http / outdated-library
        // signals, then reassemble the response from the collected
        // bytes. Chunked responses (no content-length) pass through
        // unchanged - analyzing them would require streaming parser
        // work we're not going to land in this phase.
        const MAX_BODY_BYTES: usize = 512 * 1024;
        let is_html = content_type
            .as_deref()
            .map(|c| c.to_ascii_lowercase().contains("text/html"))
            .unwrap_or(false);
        let content_length = headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok());
        if !is_html || content_length.is_none_or(|n| n > MAX_BODY_BYTES) {
            return res;
        }

        analyze_html_body(res, &url, self.security.clone()).await
    }
}

/// Decode, collect, analyze and re-emit an HTML response body. Split
/// out so `handle_response` stays readable. On ANY error (decoder
/// failure, body collect failure) we fall back to an empty body -
/// the header-level signals we already recorded still help the trust
/// score even if the page won't render, and errors here are rare.
async fn analyze_html_body(
    res: Response<Body>,
    page_url: &str,
    security: Arc<SecurityStore>,
) -> Response<Body> {
    use http_body_util::{BodyExt, Full};

    let decoded = match hudsucker::decode_response(res) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "decode_response failed; body analysis skipped");
            // We consumed the response via decode_response; emit empty.
            return Response::new(Body::empty());
        }
    };
    let (parts, body) = decoded.into_parts();
    let bytes = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            tracing::debug!(error = %e, "body collect failed; body analysis skipped");
            return Response::from_parts(parts, Body::empty());
        }
    };

    // Parse + record signals. The HTML string is UTF-8-lossy decoded
    // so a mis-encoded page still produces *some* parse output.
    if let Ok(parsed) = url::Url::parse(page_url) {
        let page_host = parsed
            .host_str()
            .map(|h| h.to_ascii_lowercase())
            .unwrap_or_default();
        let page_scheme = parsed.scheme().to_string();
        if !page_host.is_empty() {
            let html = String::from_utf8_lossy(&bytes);
            let findings = crate::body_analysis::analyze(&html, &page_host, &page_scheme);
            if findings.any_signal() {
                security.record_body(&page_host, findings);
            }
        }
    }

    // Rebuild the response with the exact bytes we just analyzed. The
    // decoder strips Content-Encoding already, so downstream can treat
    // the body as plain bytes.
    let mut out = Response::from_parts(parts, Body::from(Full::new(bytes.clone())));
    // Update Content-Length to match the decoded body size; otherwise
    // the client sees the original (possibly-gzipped) length and waits
    // for bytes that aren't coming.
    out.headers_mut()
        .insert("content-length", bytes.len().to_string().parse().unwrap());
    // Content-Encoding is also gone after decode_response - reflect that
    // on the wire so the client doesn't try to re-decode the plain body.
    out.headers_mut().remove("content-encoding");
    out
}

/// Handle to a running proxy. Dropping the runner or calling `shutdown` stops it.
/// Kept around so a future "restart proxy" command can cleanly reuse the port.
#[allow(dead_code)]
pub struct ProxyRunner {
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<()>,
}

impl ProxyRunner {
    #[allow(dead_code)]
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.join.await;
    }
}

/// What the MITM proxy's upstream HTTP client is wired to.
#[derive(Clone)]
pub enum Upstream {
    /// Direct internet via rustls.
    Direct,
    /// Tunnel every connection through a SOCKS5 endpoint (external Tor / ssh / etc).
    Socks5(SocketAddr),
    /// Dial every target through the embedded arti `TorClient`.
    #[cfg(feature = "built-in-tor")]
    BuiltInTor(crate::embedded_tor::SharedTor),
}

/// Start the proxy on the requested port using the supplied CA to sign leaf certs.
#[allow(clippy::too_many_arguments)]
pub async fn start(
    port: u16,
    ca: RootCa,
    filters: Arc<RwLock<Arc<Vec<RegexSet>>>>,
    filters_enabled: Arc<AtomicBool>,
    stats: Arc<ProxyStats>,
    block_log: Arc<BlockLog>,
    security: Arc<SecurityStore>,
    upstream: Upstream,
) -> anyhow::Result<ProxyRunner> {
    let cert_params = ca.cert_params().context("building CA params for proxy")?;
    let cert = cert_params
        .self_signed(&ca.key_pair)
        .context("self-signing CA for proxy")?;

    let authority = RcgenAuthority::new(ca.key_pair, cert, 1_000);

    let handler = BlueFlameHandler {
        filters: filters.clone(),
        filters_enabled: filters_enabled.clone(),
        stats: stats.clone(),
        block_log: block_log.clone(),
        security: security.clone(),
        pending_url: None,
    };

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    // Build a concrete client per upstream variant - hudsucker's builder
    // typestate needs the full connector type resolved in each arm.
    // Every arm threads the SAME custom `ClientConfig` in so outbound
    // TLS verification captures the server cert via our capture-verifier
    // regardless of which upstream transport the user picked.
    use hyper_util::client::legacy::{connect::HttpConnector, Client};
    use hyper_util::rt::TokioExecutor;

    let builder = Proxy::builder().with_addr(addr);
    let tls_config = crate::tls_verifier::client_config(security.clone());

    let join = match upstream {
        Upstream::Socks5(socks) => {
            tracing::info!(?socks, "proxy upstream routed through SOCKS5");
            let https = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls_config)
                .https_or_http()
                .enable_http1()
                .wrap_connector(crate::socks_connector::Socks5Connector::new(socks));
            let client: Client<_, hudsucker::Body> = Client::builder(TokioExecutor::new())
                .http1_title_case_headers(true)
                .http1_preserve_header_case(true)
                .build(https);
            let proxy = builder
                .with_client(client)
                .with_ca(authority)
                .with_http_handler(handler)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .build();
            tokio::spawn(async move {
                if let Err(e) = proxy.start().await {
                    tracing::error!(error = ?e, "proxy task exited with error");
                }
            })
        }
        #[cfg(feature = "built-in-tor")]
        Upstream::BuiltInTor(tor) => {
            tracing::info!("proxy upstream routed through embedded Tor (arti)");
            let https = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls_config)
                .https_or_http()
                .enable_http1()
                .wrap_connector(crate::embedded_tor::TorBuiltInConnector::new(tor));
            let client: Client<_, hudsucker::Body> = Client::builder(TokioExecutor::new())
                .http1_title_case_headers(true)
                .http1_preserve_header_case(true)
                .build(https);
            let proxy = builder
                .with_client(client)
                .with_ca(authority)
                .with_http_handler(handler)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .build();
            tokio::spawn(async move {
                if let Err(e) = proxy.start().await {
                    tracing::error!(error = ?e, "proxy task exited with error");
                }
            })
        }
        Upstream::Direct => {
            // Switched away from hudsucker's `.with_rustls_client()` so
            // we can inject the capture-verifier TLS config. Mirrors the
            // SOCKS5 arm but uses the default hyper `HttpConnector`.
            let mut http = HttpConnector::new();
            http.enforce_http(false);
            let https = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls_config)
                .https_or_http()
                .enable_http1()
                .wrap_connector(http);
            let client: Client<_, hudsucker::Body> = Client::builder(TokioExecutor::new())
                .http1_title_case_headers(true)
                .http1_preserve_header_case(true)
                .build(https);
            let proxy = builder
                .with_client(client)
                .with_ca(authority)
                .with_http_handler(handler)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .build();
            tokio::spawn(async move {
                if let Err(e) = proxy.start().await {
                    tracing::error!(error = ?e, "proxy task exited with error");
                }
            })
        }
    };

    tracing::info!(port, "proxy started");
    Ok(ProxyRunner {
        shutdown_tx: Some(shutdown_tx),
        join,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filters_match_known_trackers() {
        let rs = RegexSet::new(default_filter_patterns()).unwrap();
        assert!(rs.is_match("https://www.google-analytics.com/collect?v=1"));
        assert!(rs.is_match("https://stats.doubleclick.net/dc.js"));
        assert!(rs.is_match("https://connect.facebook.com/tr?id=123"));
    }

    #[test]
    fn default_filters_let_regular_traffic_through() {
        let rs = RegexSet::new(default_filter_patterns()).unwrap();
        assert!(!rs.is_match("https://example.com/index.html"));
        assert!(!rs.is_match("https://github.com/w1ck3ds0d4/BlueFlame"));
    }

    #[test]
    fn stats_snapshot_is_stable() {
        let stats = ProxyStats::default();
        stats.requests_total.store(10, Ordering::Relaxed);
        stats.requests_blocked.store(3, Ordering::Relaxed);
        stats.bytes_saved.store(6144, Ordering::Relaxed);
        let snap = stats.snapshot();
        assert_eq!(snap.requests_total, 10);
        assert_eq!(snap.requests_blocked, 3);
        assert_eq!(snap.bytes_saved, 6144);
    }
}
