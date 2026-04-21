//! Composite trust scoring for the page in the URL bar.
//!
//! Signals are bucketed into three categories the UI surfaces separately:
//! - **malware**: reputation hits (known-bad host list), suspicious
//!   resource origins, binary / unusual download patterns.
//! - **scam**: URL-shape phishing heuristics (punycode, IP literals,
//!   excessive subdomains, login-form on HTTP).
//! - **vuln**: transport-security posture (HTTP vs HTTPS, mixed content,
//!   eventually missing response headers when the response hook lands).
//!
//! Each category starts at 100 (clean) and is decreased by its signals.
//! The unified top-level `score` is the MIN across categories - if any
//! category lands in "danger" the badge shows danger regardless of how
//! clean the others look. The category structs are what the drill-down
//! panels render; the top-level `signals` is the flat concatenation the
//! overview tab renders.
//!
//! Response-side signals (TLS cert, security headers, body analysis)
//! aren't plumbed yet - the MITM proxy currently only intercepts
//! requests. Phase 2 extends `BlueFlameHandler` with a response hook.

use serde::Serialize;

use crate::reputation::ReputationStore;
use crate::security::HostSignals;

/// TLDs that disproportionately host phishing / malware per public
/// abuse registrars' reports. Hitting one is a moderate nudge, not a
/// zero, because the TLD itself doesn't make a site malicious.
const SUSPICIOUS_TLDS: &[&str] = &[
    "zip", "mov", "click", "country", "gq", "cf", "ml", "tk", "top", "xyz", "work",
];

/// Common brand names attackers impersonate. Any host that contains one
/// of these as a SUBDOMAIN label but whose effective root domain doesn't
/// match is flagged as likely brand impersonation.
const PROTECTED_BRAND_LABELS: &[(&str, &str)] = &[
    ("paypal", "paypal.com"),
    ("google", "google.com"),
    ("microsoft", "microsoft.com"),
    ("apple", "apple.com"),
    ("amazon", "amazon.com"),
    ("facebook", "facebook.com"),
    ("instagram", "instagram.com"),
    ("netflix", "netflix.com"),
];

/// Canonical "confusable" character swaps attackers use to spell protected
/// brands with look-alikes. Normalising the host through this map and
/// testing against the protected brand list catches `paypa1`, `g00gle`,
/// `faceb0ok`, `microsft`, etc. when the exact-substring brand check
/// doesn't. Not exhaustive - IDN homoglyph attacks use Unicode letters
/// that look like ASCII ones, covered separately by the `punycode` signal.
fn deconfuse(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | 'l' => 'l',
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '7' => 't',
            '$' => 's',
            '@' => 'a',
            _ => c,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrustAssessment {
    /// 0-100 unified composite. MIN of the three category scores so the
    /// worst category dominates the badge in the URL bar.
    pub score: u8,
    /// Bucketed label for the UI: `trusted` / `ok` / `suspect` / `danger`.
    pub label: String,
    /// Flat list of every signal across categories for the overview tab.
    pub signals: Vec<TrustSignal>,
    /// Per-category breakdown for the drill-down panels.
    pub categories: TrustCategories,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrustCategories {
    pub malware: TrustCategory,
    pub scam: TrustCategory,
    pub vuln: TrustCategory,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrustCategory {
    /// Stable id: `malware` / `scam` / `vuln`. Drives styling + routing.
    pub key: String,
    /// Human-readable category name.
    pub name: String,
    pub score: u8,
    pub label: String,
    pub signals: Vec<TrustSignal>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrustSignal {
    pub id: String,
    pub message: String,
    /// `positive` / `neutral` / `negative` - drives the color in the UI.
    pub kind: String,
    /// `malware` / `scam` / `vuln` - which category this signal belongs to.
    pub category: String,
}

pub fn evaluate(
    url: &str,
    blocks_on_host: usize,
    host_signals: &HostSignals,
    reputation: &ReputationStore,
) -> TrustAssessment {
    let parsed = url::Url::parse(url).ok();
    let host = parsed
        .as_ref()
        .and_then(|u| u.host_str())
        .map(|h| h.to_ascii_lowercase())
        .unwrap_or_default();
    let scheme = parsed
        .as_ref()
        .map(|u| u.scheme().to_string())
        .unwrap_or_default();

    let malware = score_malware(&host, host_signals, reputation);
    let scam = score_scam(&host, host_signals);
    let vuln = score_vuln(&scheme, host_signals, blocks_on_host);

    let unified = malware.score.min(scam.score).min(vuln.score);
    let signals: Vec<TrustSignal> = malware
        .signals
        .iter()
        .chain(scam.signals.iter())
        .chain(vuln.signals.iter())
        .cloned()
        .collect();

    TrustAssessment {
        score: unified,
        label: bucket(unified),
        signals,
        categories: TrustCategories {
            malware,
            scam,
            vuln,
        },
    }
}

fn score_malware(
    host: &str,
    host_signals: &HostSignals,
    reputation: &ReputationStore,
) -> TrustCategory {
    let mut score: i32 = 100;
    let mut signals: Vec<TrustSignal> = Vec::new();

    if !host.is_empty() && reputation.is_known_bad(host) {
        score = 0;
        signals.push(signal(
            "known-malicious",
            format!("{host} is on a reputation blocklist (bundled / URLHaus / etc)"),
            "negative",
            "malware",
        ));
    }

    if host_signals.requests_blocked >= 10 {
        score -= 15;
        signals.push(signal(
            "heavy-blocking",
            format!(
                "{} requests from this page matched the filter - heavy third-party footprint",
                host_signals.requests_blocked
            ),
            "negative",
            "malware",
        ));
    }

    // Suspicious TLD nudge. Not enough on its own to flag malware, but
    // compounds with the other signals when several fire together.
    if let Some(tld) = host.rsplit('.').next() {
        if SUSPICIOUS_TLDS.contains(&tld) {
            score -= 15;
            signals.push(signal(
                "suspicious-tld",
                format!(".{tld} is over-represented in phishing / malware reports"),
                "negative",
                "malware",
            ));
        }
    }

    if signals.is_empty() {
        signals.push(signal(
            "no-known-bad",
            "host is not on the local known-bad reputation list".into(),
            "positive",
            "malware",
        ));
    }

    finalize("malware", "malware / reputation", score, signals)
}

fn score_scam(host: &str, host_signals: &HostSignals) -> TrustCategory {
    let mut score: i32 = 100;
    let mut signals: Vec<TrustSignal> = Vec::new();

    // Cross-origin login form is a very strong phishing signal - the
    // classic shape of a credential-collecting fake login page.
    if let Some(tgt) = &host_signals.body.login_form_cross_origin_host {
        score -= 50;
        signals.push(signal(
            "cross-origin-login-form",
            format!("login form posts password to {tgt} - different host than the page"),
            "negative",
            "scam",
        ));
    }

    if host.is_empty() {
        return finalize("scam", "scam / phishing", score, signals);
    }

    let ip_literal = host.parse::<std::net::IpAddr>().is_ok();
    if ip_literal {
        score -= 30;
        signals.push(signal(
            "ip-literal",
            "host is a raw IP address - unusual for legitimate sites".into(),
            "negative",
            "scam",
        ));
    }

    let subdomain_depth = host.matches('.').count();
    if subdomain_depth >= 4 {
        score -= 15;
        signals.push(signal(
            "many-subdomains",
            format!(
                "{subdomain_depth} levels of subdomain - sometimes used to hide the real origin"
            ),
            "negative",
            "scam",
        ));
    }

    if host.contains("xn--") {
        score -= 30;
        signals.push(signal(
            "punycode",
            "host uses punycode (Unicode in the name) - check it's the brand you expect".into(),
            "negative",
            "scam",
        ));
    }

    // Brand impersonation: protected brand label appears in a subdomain
    // but the effective root domain doesn't match. Also run the check
    // against a de-confused version of the host to catch `paypa1`,
    // `g00gle`, `micros0ft` etc. - attackers swap digits for letters
    // that look identical in most fonts.
    let deconfused = deconfuse(host);
    for (brand, real_root) in PROTECTED_BRAND_LABELS {
        let matches_on_host = host
            .split('.')
            .any(|part| part.contains(brand) && part != *brand && !host.ends_with(real_root));
        let matches_on_deconfused = deconfused != host
            && deconfused
                .split('.')
                .any(|part| part.contains(brand) && !deconfused.ends_with(real_root));
        if matches_on_host {
            score -= 35;
            signals.push(signal(
                "brand-impersonation",
                format!("host mentions '{brand}' but root domain is not {real_root}"),
                "negative",
                "scam",
            ));
            break;
        }
        if matches_on_deconfused {
            score -= 45;
            signals.push(signal(
                "homoglyph",
                format!(
                    "host looks like '{brand}' spelled with digit / symbol look-alikes - likely impersonation"
                ),
                "negative",
                "scam",
            ));
            break;
        }
    }

    // Hyphen-heavy hosts are a weak phishing signal on their own but
    // compound with other red flags.
    let hyphen_count = host.matches('-').count();
    if hyphen_count >= 3 {
        score -= 10;
        signals.push(signal(
            "hyphen-heavy",
            format!("{hyphen_count} hyphens in the hostname - unusual for real brands"),
            "negative",
            "scam",
        ));
    }

    if signals.is_empty() {
        signals.push(signal(
            "clean-url-shape",
            "no URL-shape phishing signals on this host".into(),
            "positive",
            "scam",
        ));
    }

    finalize("scam", "scam / phishing", score, signals)
}

fn score_vuln(scheme: &str, host_signals: &HostSignals, blocks_on_host: usize) -> TrustCategory {
    let mut score: i32 = 100;
    let mut signals: Vec<TrustSignal> = Vec::new();

    if scheme == "http" {
        score -= 60;
        signals.push(signal(
            "http",
            "connection is plain HTTP - no transport encryption".into(),
            "negative",
            "vuln",
        ));
    } else if scheme == "https" {
        signals.push(signal(
            "https",
            "connection is HTTPS - transport is encrypted".into(),
            "positive",
            "vuln",
        ));
    }

    if host_signals.mixed_content {
        score -= 30;
        signals.push(signal(
            "mixed-content",
            "page loaded at least one resource over plain HTTP".into(),
            "negative",
            "vuln",
        ));
    }

    // Response headers. Only score them once we've actually seen an
    // HTML response for this host - otherwise "missing everything" on
    // every fresh nav would dominate the score with false positives.
    if host_signals.headers.seen {
        let h = &host_signals.headers;
        if !h.csp {
            score -= 15;
            signals.push(signal(
                "missing-csp",
                "no Content-Security-Policy header - page has no declared script origins".into(),
                "negative",
                "vuln",
            ));
        }
        // HSTS only matters over HTTPS.
        if scheme == "https" && !h.hsts {
            score -= 15;
            signals.push(signal(
                "missing-hsts",
                "no Strict-Transport-Security header - browser can be downgraded to HTTP".into(),
                "negative",
                "vuln",
            ));
        }
        if !h.xfo {
            score -= 10;
            signals.push(signal(
                "missing-xfo",
                "no X-Frame-Options header - page could be framed by a malicious site".into(),
                "negative",
                "vuln",
            ));
        }
        if !h.xcto {
            score -= 5;
            signals.push(signal(
                "missing-xcto",
                "no X-Content-Type-Options header - MIME-sniffing is allowed".into(),
                "negative",
                "vuln",
            ));
        }
        if !h.referrer_policy {
            score -= 5;
            signals.push(signal(
                "missing-referrer-policy",
                "no Referrer-Policy header - full URL leaks on outbound links".into(),
                "negative",
                "vuln",
            ));
        }
        if !h.permissions_policy {
            score -= 5;
            signals.push(signal(
                "missing-permissions-policy",
                "no Permissions-Policy header - camera / mic / geolocation not restricted".into(),
                "negative",
                "vuln",
            ));
        }
        if h.insecure_cookie {
            score -= 15;
            signals.push(signal(
                "insecure-cookie",
                "a Set-Cookie lacked both Secure and HttpOnly - session theft risk".into(),
                "negative",
                "vuln",
            ));
        }
    }

    // TLS cert signals. The capture-verifier populates `cert.seen` only
    // after a successful outbound TLS handshake, so these only fire for
    // HTTPS hosts the proxy has actually dialled. The baseline
    // expiry/self-signed cases are what's left after webpki has done
    // its strict chain check - anything webpki rejects never reaches
    // us to score.
    if host_signals.cert.seen {
        let c = &host_signals.cert;
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        if c.self_signed {
            // Won't normally reach here because WebPkiServerVerifier
            // rejects self-signed leaves without an explicit root, but
            // we keep the signal wired for when private-CA support lands.
            score -= 25;
            signals.push(signal(
                "cert-self-signed",
                format!(
                    "cert subject == issuer ({}) - self-signed, no external CA attests to the identity",
                    c.issuer_cn
                ),
                "negative",
                "vuln",
            ));
        }

        let until_expiry = c.not_after - now_secs;
        if until_expiry <= 0 {
            score -= 30;
            signals.push(signal(
                "cert-expired",
                "server cert has passed its notAfter date".into(),
                "negative",
                "vuln",
            ));
        } else if until_expiry < 7 * 86_400 {
            score -= 15;
            signals.push(signal(
                "cert-expires-soon",
                format!(
                    "server cert expires in {} days - operators sometimes forget to renew",
                    until_expiry / 86_400
                ),
                "negative",
                "vuln",
            ));
        }

        // Wildly long-lived certs haven't been legal for public CAs
        // since the CA/B forum tightened the rules in 2020 - anything
        // valid for more than ~400 days is either an internal CA or a
        // misconfigured one.
        let lifetime = c.not_after - c.not_before;
        if lifetime > 398 * 86_400 {
            score -= 10;
            signals.push(signal(
                "cert-overlong",
                format!(
                    "cert is valid for {} days - public CAs cap lifetimes at ~398d",
                    lifetime / 86_400
                ),
                "negative",
                "vuln",
            ));
        }

        if !c.issuer_cn.is_empty() {
            signals.push(signal(
                "cert-issuer",
                format!("issued by {}", c.issuer_cn),
                "neutral",
                "vuln",
            ));
        }
    }

    // Body-level signals from the HTML document. These overlap
    // categories: a cross-origin login form is BOTH a scam signal
    // (phishing shape) and a vuln signal (password trust violation).
    // We surface the heaviest ones under vuln; the scam category also
    // gets a mirror below via `score_scam_body`.
    let body = &host_signals.body;
    if body.seen {
        if body.password_on_insecure_origin {
            score -= 40;
            signals.push(signal(
                "password-on-http",
                "page has a password field served over plain HTTP".into(),
                "negative",
                "vuln",
            ));
        }
        for lib in &body.outdated_libraries {
            score -= 10;
            signals.push(signal(
                "outdated-lib",
                format!("loads an outdated JS library: {lib}"),
                "negative",
                "vuln",
            ));
        }
    }

    let third_parties = host_signals.third_party_hosts.len();
    if third_parties > 0 {
        signals.push(signal(
            "third-parties",
            format!("{third_parties} third-party origins observed from this page"),
            if third_parties > 20 {
                "negative"
            } else {
                "neutral"
            },
            "vuln",
        ));
        if third_parties > 20 {
            score -= 10;
        }
    }

    if blocks_on_host > 0 {
        signals.push(signal(
            "blocked",
            format!("{blocks_on_host} trackers blocked on this site"),
            "positive",
            "vuln",
        ));
    }

    signals.push(signal(
        "proxy",
        "traffic is routed through the BlueFlame filter proxy".into(),
        "positive",
        "vuln",
    ));

    finalize("vuln", "vulnerabilities / transport", score, signals)
}

fn finalize(key: &str, name: &str, score: i32, signals: Vec<TrustSignal>) -> TrustCategory {
    let score = score.clamp(0, 100) as u8;
    TrustCategory {
        key: key.into(),
        name: name.into(),
        score,
        label: bucket(score),
        signals,
    }
}

fn bucket(score: u8) -> String {
    match score {
        0..=20 => "danger",
        21..=50 => "suspect",
        51..=80 => "ok",
        _ => "trusted",
    }
    .to_string()
}

fn signal(id: &str, message: String, kind: &str, category: &str) -> TrustSignal {
    TrustSignal {
        id: id.into(),
        message,
        kind: kind.into(),
        category: category.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> HostSignals {
        HostSignals::default()
    }

    fn rep() -> ReputationStore {
        ReputationStore::with_bundled()
    }

    #[test]
    fn https_clean_site_is_trusted() {
        let a = evaluate("https://duckduckgo.com/?q=rust", 8, &empty(), &rep());
        assert_eq!(a.label, "trusted");
        assert!(a.categories.vuln.signals.iter().any(|s| s.id == "https"));
    }

    #[test]
    fn plain_http_drops_vuln_score() {
        let a = evaluate("http://example.com/", 0, &empty(), &rep());
        assert!(a.categories.vuln.score < 60);
        // Unified takes the min, so HTTP alone drops the overall score too.
        assert!(a.score < 60);
    }

    #[test]
    fn known_bad_host_zeros_unified_via_malware() {
        let a = evaluate("https://phishing.example.test/login", 0, &empty(), &rep());
        assert_eq!(a.score, 0);
        assert_eq!(a.label, "danger");
        assert_eq!(a.categories.malware.score, 0);
    }

    #[test]
    fn punycode_is_a_scam_signal() {
        let a = evaluate("https://xn--e1afmkfd.example/", 0, &empty(), &rep());
        assert!(a.categories.scam.signals.iter().any(|s| s.id == "punycode"));
        assert!(a.categories.scam.score < 100);
    }

    #[test]
    fn brand_impersonation_is_flagged() {
        // "paypal" appears as a subdomain label on a host that isn't paypal.com
        let a = evaluate("https://paypal-login.evil.example/", 0, &empty(), &rep());
        assert!(a
            .categories
            .scam
            .signals
            .iter()
            .any(|s| s.id == "brand-impersonation"));
        assert!(a.categories.scam.score < 80);
    }

    #[test]
    fn mixed_content_hits_vuln() {
        let sig = HostSignals {
            mixed_content: true,
            ..Default::default()
        };
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .any(|s| s.id == "mixed-content"));
    }

    fn with_seen_headers(
        csp: bool,
        hsts: bool,
        xfo: bool,
        xcto: bool,
        rp: bool,
        pp: bool,
        insecure_cookie: bool,
    ) -> HostSignals {
        HostSignals {
            headers: crate::security::HeaderSnapshot {
                seen: true,
                csp,
                hsts,
                xfo,
                xcto,
                referrer_policy: rp,
                permissions_policy: pp,
                insecure_cookie,
            },
            ..Default::default()
        }
    }

    #[test]
    fn missing_headers_drop_vuln_score() {
        let sig = with_seen_headers(false, false, false, false, false, false, false);
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        let vuln = &a.categories.vuln;
        assert!(vuln.signals.iter().any(|s| s.id == "missing-csp"));
        assert!(vuln.signals.iter().any(|s| s.id == "missing-hsts"));
        assert!(vuln.signals.iter().any(|s| s.id == "missing-xfo"));
        assert!(vuln.signals.iter().any(|s| s.id == "missing-xcto"));
        assert!(vuln
            .signals
            .iter()
            .any(|s| s.id == "missing-referrer-policy"));
        assert!(vuln
            .signals
            .iter()
            .any(|s| s.id == "missing-permissions-policy"));
        // 15 + 15 + 10 + 5 + 5 + 5 = 55 point drop.
        assert!(vuln.score <= 45);
    }

    #[test]
    fn all_headers_present_is_clean_vuln() {
        let sig = with_seen_headers(true, true, true, true, true, true, false);
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .all(|s| !s.id.starts_with("missing-")));
        assert_eq!(a.categories.vuln.score, 100);
    }

    #[test]
    fn insecure_cookie_is_flagged() {
        let sig = with_seen_headers(true, true, true, true, true, true, true);
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .any(|s| s.id == "insecure-cookie"));
    }

    #[test]
    fn unseen_headers_do_not_penalize() {
        // No HeaderSnapshot.seen == no header signals, regardless of missing headers.
        let a = evaluate("https://site.example/", 0, &HostSignals::default(), &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .all(|s| !s.id.starts_with("missing-")));
    }

    #[test]
    fn hsts_only_penalizes_https() {
        // Plain-HTTP gets the big http penalty but shouldn't ALSO get missing-hsts,
        // which is meaningless on HTTP anyway.
        let sig = with_seen_headers(true, false, true, true, true, true, false);
        let a = evaluate("http://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .all(|s| s.id != "missing-hsts"));
    }

    #[test]
    fn homoglyph_digit_swap_is_flagged() {
        // `1` in place of `l` - same visual shape in most fonts.
        let a = evaluate("https://paypa1.example/", 0, &empty(), &rep());
        assert!(a
            .categories
            .scam
            .signals
            .iter()
            .any(|s| s.id == "homoglyph"));
        assert!(a.categories.scam.score < 60);
    }

    #[test]
    fn homoglyph_zero_for_o_is_flagged() {
        let a = evaluate("https://g00gle.evil.example/", 0, &empty(), &rep());
        assert!(a
            .categories
            .scam
            .signals
            .iter()
            .any(|s| s.id == "homoglyph"));
    }

    #[test]
    fn suspicious_tld_nudges_malware() {
        let a = evaluate("https://some-site.zip/", 0, &empty(), &rep());
        assert!(a
            .categories
            .malware
            .signals
            .iter()
            .any(|s| s.id == "suspicious-tld"));
        assert!(a.categories.malware.score < 100);
    }

    fn with_cert(
        seen: bool,
        issuer: &str,
        subject: &str,
        not_before: i64,
        not_after: i64,
        self_signed: bool,
    ) -> HostSignals {
        HostSignals {
            cert: crate::security::CertSnapshot {
                seen,
                issuer_cn: issuer.into(),
                subject_cn: subject.into(),
                not_before,
                not_after,
                sig_alg: "1.2.840.113549.1.1.11".into(),
                self_signed,
            },
            ..Default::default()
        }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    #[test]
    fn cert_expires_soon_is_flagged() {
        let sig = with_cert(
            true,
            "Let's Encrypt",
            "site.example",
            now() - 86_400,
            now() + 3 * 86_400, // 3 days until expiry
            false,
        );
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .any(|s| s.id == "cert-expires-soon"));
    }

    #[test]
    fn cert_expired_is_penalized() {
        let sig = with_cert(
            true,
            "Let's Encrypt",
            "site.example",
            now() - 30 * 86_400,
            now() - 86_400, // expired yesterday
            false,
        );
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .any(|s| s.id == "cert-expired"));
    }

    #[test]
    fn self_signed_cert_is_flagged() {
        let sig = with_cert(
            true,
            "site.example",
            "site.example",
            now() - 86_400,
            now() + 90 * 86_400,
            true,
        );
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .any(|s| s.id == "cert-self-signed"));
    }

    #[test]
    fn overlong_cert_is_flagged() {
        let sig = with_cert(
            true,
            "Private CA",
            "site.example",
            now() - 100 * 86_400,
            now() + 500 * 86_400, // 500-day cert
            false,
        );
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .any(|s| s.id == "cert-overlong"));
    }

    #[test]
    fn cert_not_seen_does_not_emit_cert_signals() {
        let a = evaluate("https://site.example/", 0, &empty(), &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .all(|s| !s.id.starts_with("cert-")));
    }

    #[test]
    fn healthy_cert_only_emits_issuer_info() {
        let sig = with_cert(
            true,
            "Let's Encrypt",
            "site.example",
            now() - 30 * 86_400,
            now() + 60 * 86_400, // 60 days left
            false,
        );
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        let cert_signals: Vec<_> = a
            .categories
            .vuln
            .signals
            .iter()
            .filter(|s| s.id.starts_with("cert-"))
            .collect();
        // Only `cert-issuer` (neutral info) should be present.
        assert_eq!(cert_signals.len(), 1);
        assert_eq!(cert_signals[0].id, "cert-issuer");
    }

    #[test]
    fn password_on_http_page_hits_vuln() {
        let sig = HostSignals {
            body: crate::security::BodySnapshot {
                seen: true,
                password_on_insecure_origin: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let a = evaluate("http://site.example/login", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .any(|s| s.id == "password-on-http"));
    }

    #[test]
    fn cross_origin_login_form_hits_scam() {
        let sig = HostSignals {
            body: crate::security::BodySnapshot {
                seen: true,
                login_form_cross_origin_host: Some("attacker.example".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let a = evaluate("https://legit.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .scam
            .signals
            .iter()
            .any(|s| s.id == "cross-origin-login-form"));
        assert!(a.categories.scam.score < 60);
    }

    #[test]
    fn outdated_library_hits_vuln() {
        let sig = HostSignals {
            body: crate::security::BodySnapshot {
                seen: true,
                outdated_libraries: ["jQuery 1.7 (EOL)".to_string()].into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let a = evaluate("https://site.example/", 0, &sig, &rep());
        assert!(a
            .categories
            .vuln
            .signals
            .iter()
            .any(|s| s.id == "outdated-lib"));
    }

    #[test]
    fn reputation_hit_added_at_runtime_is_caught() {
        // Host that's clean under the bundled list alone...
        let empty_rep = ReputationStore::default();
        let clean = evaluate("https://freshly-reported.example/", 0, &empty(), &empty_rep);
        assert_eq!(clean.categories.malware.score, 100);
        // ...and then a feed refresh adds it - the very next evaluation
        // should flip to danger without any other wiring.
        empty_rep.extend(["freshly-reported.example".to_string()]);
        let flagged = evaluate("https://freshly-reported.example/", 0, &empty(), &empty_rep);
        assert_eq!(flagged.categories.malware.score, 0);
        assert!(flagged
            .categories
            .malware
            .signals
            .iter()
            .any(|s| s.id == "known-malicious"));
    }

    #[test]
    fn unified_is_min_of_categories() {
        // HTTP (vuln danger) + otherwise-clean host: unified must follow vuln.
        let a = evaluate("http://clean.example/", 0, &empty(), &rep());
        assert_eq!(a.score, a.categories.vuln.score);
    }

    #[test]
    fn ip_literal_is_a_scam_signal() {
        let a = evaluate("https://93.184.216.34/", 0, &empty(), &rep());
        assert!(a
            .categories
            .scam
            .signals
            .iter()
            .any(|s| s.id == "ip-literal"));
    }
}
