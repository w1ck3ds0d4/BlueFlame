//! Upstream TLS verifier that taps the server cert for trust-score
//! signals while still enforcing real chain verification via webpki.
//!
//! The proxy terminates TLS from the webview (using its own MITM CA to
//! sign leaf certs) and re-establishes TLS outbound. We only get to
//! introspect the real site's cert on that OUTBOUND handshake. By
//! wrapping rustls' `WebPkiServerVerifier` we stay as strict as the
//! default client-side Rust TLS stack - if a cert is expired, revoked,
//! or doesn't chain, verification still fails and the connection dies.
//! On success we parse the leaf cert and stash issuer / subject /
//! validity window in `SecurityStore`, which `trust::evaluate` reads.
//!
//! Intentionally avoids any "dangerous" bypasses. This is not a place
//! to loosen verification for self-signed hosts - keep the security
//! posture identical to stock rustls, only add observability.
//!
//! The parser (`x509-parser`) is the rusticata crate. We only extract
//! a handful of fields; failure to parse is non-fatal - we just skip
//! the signal record and let the verification result speak for itself.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::security::{CertSnapshot, SecurityStore};

#[derive(Debug)]
pub struct CaptureVerifier {
    inner: Arc<WebPkiServerVerifier>,
    store: Arc<SecurityStore>,
}

impl CaptureVerifier {
    pub fn new(store: Arc<SecurityStore>) -> Arc<Self> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let inner = WebPkiServerVerifier::builder(Arc::new(roots))
            .build()
            .expect("building webpki verifier");
        Arc::new(Self { inner, store })
    }
}

impl ServerCertVerifier for CaptureVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let result = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );

        // Only record on successful verification. A failure means the
        // connection won't carry traffic, so there's no page for the
        // user to land on and nothing to score. Failures still surface
        // via the browser's own error UI.
        if result.is_ok() {
            if let Some(snapshot) = parse_cert(end_entity) {
                let host = server_name_host(server_name);
                if !host.is_empty() {
                    self.store.record_cert(&host, snapshot);
                }
            }
        }
        result
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// `ServerName::DnsName(...)` for normal TLS handshakes, the IP variants
/// for raw-IP hosts. We only stash signals for the DNS case - an IP URL
/// is already flagged by the scam scorer elsewhere.
fn server_name_host(name: &ServerName<'_>) -> String {
    match name {
        ServerName::DnsName(d) => d.as_ref().to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Extract the fields we care about from a DER-encoded leaf cert. All
/// we need is the issuer CN, subject CN, validity window, signature
/// algorithm OID. Everything else on the cert we ignore - the UI only
/// shows a handful of fields and the scorer only reads the validity
/// dates + self-signed heuristic.
fn parse_cert(der: &CertificateDer<'_>) -> Option<CertSnapshot> {
    use x509_parser::prelude::*;

    let (_, cert) = X509Certificate::from_der(der.as_ref()).ok()?;

    let issuer_cn = common_name(cert.issuer());
    let subject_cn = common_name(cert.subject());
    let validity = cert.validity();
    let not_before = validity.not_before.timestamp();
    let not_after = validity.not_after.timestamp();
    let sig_alg = cert.signature_algorithm.oid().to_id_string();
    let self_signed = cert.issuer() == cert.subject();

    Some(CertSnapshot {
        seen: true,
        issuer_cn,
        subject_cn,
        not_before,
        not_after,
        sig_alg,
        self_signed,
    })
}

fn common_name(name: &x509_parser::x509::X509Name) -> String {
    name.iter_common_name()
        .filter_map(|cn| cn.as_str().ok())
        .next()
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Build a `ClientConfig` that uses the capture verifier. Webpki-roots
/// is the trust anchor, matching `hyper_rustls::with_webpki_roots()`.
/// ALPN is left empty so `hyper_rustls` can fill it in based on which
/// HTTP versions it was told to enable.
pub fn client_config(store: Arc<SecurityStore>) -> ClientConfig {
    let verifier = CaptureVerifier::new(store);
    ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_name_dns_extracts_lowercase_host() {
        let n = ServerName::try_from("Example.COM").unwrap();
        assert_eq!(server_name_host(&n), "example.com");
    }

    #[test]
    fn server_name_ip_returns_empty() {
        let n = ServerName::try_from("127.0.0.1").unwrap();
        assert!(server_name_host(&n).is_empty());
    }
}
