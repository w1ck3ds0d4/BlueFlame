//! Root CA cert management for the MITM proxy.
//!
//! On first run BlueFlame generates a self-signed root CA. The cert is
//! written to the user data dir so it can be installed into the OS trust
//! store. The private key also lives on disk so the proxy can re-load it
//! across restarts without regenerating the CA (which would break any
//! already-installed trust).

use std::path::{Path, PathBuf};

use anyhow::Context;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

const CA_COMMON_NAME: &str = "BlueFlame Root CA";
const CA_ORG: &str = "BlueFlame";
const CERT_FILE: &str = "blueflame-ca.crt";
const KEY_FILE: &str = "blueflame-ca.key";

/// Loaded or freshly generated root CA usable by the MITM proxy.
pub struct RootCa {
    pub cert_pem: String,
    pub key_pair: KeyPair,
}

impl RootCa {
    /// Reconstruct the CA cert params so the proxy can use them to sign per-host leaf certs.
    pub fn cert_params(&self) -> anyhow::Result<CertificateParams> {
        CertificateParams::from_ca_cert_pem(&self.cert_pem).context("parsing CA cert PEM")
    }
}

/// Load the root CA from `dir`, or generate + persist a new one if missing.
pub fn load_or_create<P: AsRef<Path>>(dir: P) -> anyhow::Result<RootCa> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).context("creating CA dir")?;

    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);

    if cert_path.exists() && key_path.exists() {
        let cert_pem = std::fs::read_to_string(&cert_path).context("reading CA cert")?;
        let key_pem = std::fs::read_to_string(&key_path).context("reading CA key")?;
        let key_pair = KeyPair::from_pem(&key_pem).context("parsing CA key PEM")?;
        return Ok(RootCa { cert_pem, key_pair });
    }

    let ca = generate()?;
    std::fs::write(&cert_path, &ca.cert_pem).context("writing CA cert")?;
    std::fs::write(&key_path, ca.key_pair.serialize_pem()).context("writing CA key")?;
    Ok(ca)
}

fn generate() -> anyhow::Result<RootCa> {
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, CA_COMMON_NAME);
    params
        .distinguished_name
        .push(DnType::OrganizationName, CA_ORG);
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

    let key_pair = KeyPair::generate().context("generating CA key pair")?;
    let cert = params
        .self_signed(&key_pair)
        .context("self-signing CA cert")?;

    Ok(RootCa {
        cert_pem: cert.pem(),
        key_pair,
    })
}

/// Path to the cert file the user needs to trust in their OS.
#[allow(dead_code)] // exposed for a future "reveal in explorer" command
pub fn cert_path(dir: &Path) -> PathBuf {
    dir.join(CERT_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_reloads_same_ca() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let first = load_or_create(tmp.path()).expect("generate");
        let second = load_or_create(tmp.path()).expect("reload");
        assert_eq!(first.cert_pem, second.cert_pem, "cert should be stable");
    }

    #[test]
    fn cert_pem_parses_as_ca() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ca = load_or_create(tmp.path()).expect("generate");
        let params = CertificateParams::from_ca_cert_pem(&ca.cert_pem).expect("parse");
        assert!(matches!(params.is_ca, rcgen::IsCa::Ca(_)));
    }
}
