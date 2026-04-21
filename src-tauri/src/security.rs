//! Per-host signal stash populated by the MITM proxy.
//!
//! The proxy sees every request the webview makes. For each request we
//! attribute signals back to the PAGE host (extracted from the Referer
//! header) and accumulate them in a small in-memory map. `trust::evaluate`
//! then reads the accumulated signals for the URL the user is viewing and
//! turns them into category-scored `TrustAssessment` data for the UI.
//!
//! Phase 1 is request-side only - response headers, TLS cert info, and
//! script behavior analysis all require deeper hooks we haven't wired yet.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Default, Clone)]
pub struct HostSignals {
    /// Hosts that THIS page made requests to, excluding the page's own
    /// host. Capacity-bounded so a long browsing session can't grow
    /// this unbounded.
    pub third_party_hosts: HashSet<String>,
    /// Any resource requested over plain HTTP while the page was HTTPS.
    pub mixed_content: bool,
    /// Total requests the proxy saw attributed to this page.
    pub requests_seen: u64,
    /// Subset of those requests that matched a filter rule.
    pub requests_blocked: u64,
    /// Response-side signals captured by the proxy's `handle_response`
    /// hook. Only populated once we've seen at least one HTML response
    /// for this host.
    pub headers: HeaderSnapshot,
    /// Upstream server-cert snapshot captured by the custom rustls
    /// verifier during the outbound TLS handshake. `seen` is false
    /// until the first successful HTTPS connection to this host.
    pub cert: CertSnapshot,
    /// Findings from parsing the most recent HTML document response
    /// for this host (login forms, password inputs, outdated libs).
    pub body: BodySnapshot,
    /// Unix seconds when this host was last touched. Lets stale entries
    /// get pruned if/when we cap the map size.
    pub last_seen_at: u64,
}

/// Merged body-analysis findings for a host. We OR across multiple
/// page loads so any navigation that triggered a signal stays recorded
/// until the user explicitly resets. Populated from
/// `body_analysis::BodyFindings` by `record_body`.
#[derive(Debug, Default, Clone)]
pub struct BodySnapshot {
    pub seen: bool,
    pub login_form_cross_origin_host: Option<String>,
    pub password_on_insecure_origin: bool,
    pub outdated_libraries: HashSet<String>,
}

/// Subset of the upstream leaf cert the trust scorer cares about.
/// `seen` gates the whole snapshot - no signals fire for hosts we
/// haven't completed a TLS handshake to yet. Populated by the
/// custom rustls `ServerCertVerifier`; chain verification is still
/// performed by webpki (we never fabricate a successful verification).
///
/// `subject_cn` / `sig_alg` are populated for forward-compat with
/// UI-only surfaces (a future "cert details" row in the panel) - the
/// scorer itself doesn't read them yet.
#[derive(Debug, Default, Clone)]
#[allow(dead_code)]
pub struct CertSnapshot {
    pub seen: bool,
    pub issuer_cn: String,
    pub subject_cn: String,
    /// Unix seconds. `x509-parser` returns an `i64` for this.
    pub not_before: i64,
    pub not_after: i64,
    /// Signature algorithm OID, e.g. `1.2.840.113549.1.1.11`.
    pub sig_alg: String,
    /// `true` if issuer == subject at the DN level. webpki would reject
    /// these by default in verification, but we capture the flag in
    /// case a future option relaxes verification for the user.
    pub self_signed: bool,
}

/// Presence/absence of security-relevant response headers on the most
/// recent HTML document response observed for a host. Booleans are `true`
/// when the header WAS present - `seen` gates the whole snapshot so we
/// don't report missing-everything for hosts we haven't seen a response
/// from yet.
#[derive(Debug, Default, Clone)]
pub struct HeaderSnapshot {
    pub seen: bool,
    pub csp: bool,
    pub hsts: bool,
    pub xfo: bool,
    pub xcto: bool,
    pub referrer_policy: bool,
    pub permissions_policy: bool,
    /// `true` if any Set-Cookie on the response was missing both `Secure`
    /// and `HttpOnly` flags. A weak signal on its own but compounds with
    /// other vuln signals.
    pub insecure_cookie: bool,
}

/// Bound on `third_party_hosts` per page so a page with thousands of ad
/// requests doesn't blow up memory. New hosts over the cap are dropped.
const THIRD_PARTY_CAP: usize = 200;
/// Bound on the total number of pages tracked. When full, the oldest
/// entry (by `last_seen_at`) is evicted on the next insert.
const PAGE_CAP: usize = 512;

#[derive(Debug, Default)]
pub struct SecurityStore {
    hosts: RwLock<HashMap<String, HostSignals>>,
}

impl SecurityStore {
    /// Attribute `req_url` to the page indicated by `referer`. Without a
    /// usable Referer we can't assign the request to any page, so we
    /// drop it - the request still flows through the proxy, we just
    /// don't count it for trust scoring. `blocked` is whether the filter
    /// returned 204 on this request.
    pub fn record_request(&self, req_url: &str, referer: Option<&str>, blocked: bool) {
        let Some(page) = referer
            .and_then(|r| url::Url::parse(r).ok())
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        else {
            return;
        };

        let req = url::Url::parse(req_url).ok();
        let req_host = req
            .as_ref()
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()));
        let req_scheme = req
            .as_ref()
            .map(|u| u.scheme().to_string())
            .unwrap_or_default();

        let third_party = match req_host.as_ref() {
            Some(rh) if rh != &page => Some(rh.clone()),
            _ => None,
        };
        let mixed = req_scheme == "http";

        let mut hosts = self.hosts.write().expect("security store rwlock poisoned");
        if hosts.len() >= PAGE_CAP && !hosts.contains_key(&page) {
            evict_oldest(&mut hosts);
        }
        let entry = hosts.entry(page).or_default();
        entry.requests_seen = entry.requests_seen.saturating_add(1);
        if blocked {
            entry.requests_blocked = entry.requests_blocked.saturating_add(1);
        }
        if let Some(tp) = third_party {
            if entry.third_party_hosts.len() < THIRD_PARTY_CAP {
                entry.third_party_hosts.insert(tp);
            }
        }
        if mixed {
            entry.mixed_content = true;
        }
        entry.last_seen_at = now_secs();
    }

    /// Called from the proxy's response hook. `req_url` is the URL that
    /// originated the response (carried across from the request via the
    /// handler's pending slot). We only record headers for HTML document
    /// responses - on a page with 100 images and one HTML doc, the
    /// headers on the doc are what define the page's security policy,
    /// image subresources would just overwrite them with noise.
    #[allow(clippy::too_many_arguments)]
    pub fn record_response(
        &self,
        req_url: &str,
        content_type: Option<&str>,
        has_csp: bool,
        has_hsts: bool,
        has_xfo: bool,
        has_xcto: bool,
        has_referrer_policy: bool,
        has_permissions_policy: bool,
        insecure_cookie: bool,
    ) {
        let is_html = content_type
            .map(|c| c.to_ascii_lowercase().contains("text/html"))
            .unwrap_or(false);
        if !is_html {
            return;
        }
        let Some(host) = url::Url::parse(req_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        else {
            return;
        };

        let mut hosts = self.hosts.write().expect("security store rwlock poisoned");
        if hosts.len() >= PAGE_CAP && !hosts.contains_key(&host) {
            evict_oldest(&mut hosts);
        }
        let entry = hosts.entry(host).or_default();
        entry.headers = HeaderSnapshot {
            seen: true,
            csp: has_csp,
            hsts: has_hsts,
            xfo: has_xfo,
            xcto: has_xcto,
            referrer_policy: has_referrer_policy,
            permissions_policy: has_permissions_policy,
            insecure_cookie,
        };
        entry.last_seen_at = now_secs();
    }

    /// Merge body-analysis findings from an HTML document response
    /// into the host's snapshot. Signals OR in (once flagged, stays
    /// flagged for the session) - a phishing form detected on one
    /// page load should keep the warning up even if subsequent
    /// navigations don't reparse the same form.
    pub fn record_body(&self, host: &str, findings: crate::body_analysis::BodyFindings) {
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() {
            return;
        }
        let mut hosts = self.hosts.write().expect("security store rwlock poisoned");
        if hosts.len() >= PAGE_CAP && !hosts.contains_key(&host) {
            evict_oldest(&mut hosts);
        }
        let entry = hosts.entry(host).or_default();
        entry.body.seen = true;
        if findings.login_form_cross_origin.is_some() {
            entry.body.login_form_cross_origin_host = findings.login_form_cross_origin;
        }
        if findings.password_on_insecure_origin {
            entry.body.password_on_insecure_origin = true;
        }
        for lib in findings.outdated_libraries {
            entry.body.outdated_libraries.insert(lib);
        }
        entry.last_seen_at = now_secs();
    }

    /// Called from the custom rustls verifier on every successful
    /// upstream TLS handshake. Overwrites any previous cert snapshot
    /// for the host - certs rotate, so newer is more relevant.
    pub fn record_cert(&self, host: &str, cert: CertSnapshot) {
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() {
            return;
        }
        let mut hosts = self.hosts.write().expect("security store rwlock poisoned");
        if hosts.len() >= PAGE_CAP && !hosts.contains_key(&host) {
            evict_oldest(&mut hosts);
        }
        let entry = hosts.entry(host).or_default();
        entry.cert = cert;
        entry.last_seen_at = now_secs();
    }

    /// Snapshot of everything we've attributed to `host`. Returns an
    /// empty struct if we haven't seen the host yet.
    pub fn signals_for(&self, host: &str) -> HostSignals {
        let hosts = self.hosts.read().expect("security store rwlock poisoned");
        hosts
            .get(&host.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// Drop all signals. Called from the Dashboard reset button.
    #[allow(dead_code)]
    pub fn clear(&self) {
        let mut hosts = self.hosts.write().expect("security store rwlock poisoned");
        hosts.clear();
    }
}

fn evict_oldest(hosts: &mut HashMap<String, HostSignals>) {
    if let Some((victim, _)) = hosts
        .iter()
        .min_by_key(|(_, v)| v.last_seen_at)
        .map(|(k, v)| (k.clone(), v.last_seen_at))
    {
        hosts.remove(&victim);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_referer_is_a_noop() {
        let s = SecurityStore::default();
        s.record_request("https://tracker.example/x", None, false);
        assert!(s.signals_for("tracker.example").requests_seen == 0);
    }

    #[test]
    fn request_attributes_to_page_host() {
        let s = SecurityStore::default();
        s.record_request(
            "https://tracker.example/px.gif",
            Some("https://news.example/article"),
            false,
        );
        let sig = s.signals_for("news.example");
        assert_eq!(sig.requests_seen, 1);
        assert!(sig.third_party_hosts.contains("tracker.example"));
        assert!(!sig.mixed_content);
    }

    #[test]
    fn mixed_content_is_flagged() {
        let s = SecurityStore::default();
        s.record_request(
            "http://img.example/logo.png",
            Some("https://site.example/"),
            false,
        );
        assert!(s.signals_for("site.example").mixed_content);
    }

    #[test]
    fn blocked_count_tracked() {
        let s = SecurityStore::default();
        s.record_request(
            "https://ads.example/px",
            Some("https://site.example/"),
            true,
        );
        assert_eq!(s.signals_for("site.example").requests_blocked, 1);
    }

    #[test]
    fn same_host_request_is_not_third_party() {
        let s = SecurityStore::default();
        s.record_request(
            "https://site.example/js/app.js",
            Some("https://site.example/"),
            false,
        );
        assert!(s.signals_for("site.example").third_party_hosts.is_empty());
    }

    #[test]
    fn non_html_response_is_ignored() {
        let s = SecurityStore::default();
        s.record_response(
            "https://site.example/logo.png",
            Some("image/png"),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        );
        assert!(!s.signals_for("site.example").headers.seen);
    }

    #[test]
    fn html_response_records_headers() {
        let s = SecurityStore::default();
        s.record_response(
            "https://site.example/",
            Some("text/html; charset=utf-8"),
            true,
            true,
            true,
            true,
            true,
            true,
            false,
        );
        let h = s.signals_for("site.example").headers;
        assert!(h.seen);
        assert!(h.csp && h.hsts && h.xfo && h.xcto && h.referrer_policy && h.permissions_policy);
        assert!(!h.insecure_cookie);
    }
}
