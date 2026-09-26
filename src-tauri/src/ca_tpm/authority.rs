//! A `hudsucker::CertificateAuthority` for a root key that can't hand out
//! its private bytes (our TPM/CNG-backed [`super::TpmSigningKey`]).
//!
//! `hudsucker::certificate_authority::RcgenAuthority` can't be used here: it
//! reuses the *root's own* key pair as every leaf certificate's TLS key,
//! which requires exporting that key pair's private bytes into the rustls
//! `ServerConfig` it builds - impossible for a non-exportable key. Instead,
//! each newly seen host gets a fresh, ordinary in-memory ECDSA key pair;
//! only that key pair's *public* half is signed by the root (one TPM/CNG
//! call), and the resulting `ServerConfig` - leaf cert plus the leaf's own
//! exportable private key - is cached exactly like `RcgenAuthority` caches
//! its own. Pages hitting an already-seen host never touch the TPM at all;
//! only the first connection to a new host does.

use std::sync::Arc;

use http::uri::Authority;
use hudsucker::certificate_authority::CertificateAuthority;
use hudsucker::rustls::crypto::CryptoProvider;
use hudsucker::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use hudsucker::rustls::ServerConfig;
use moka::future::Cache;
use rcgen::{CertificateParams, DistinguishedName, DnType, Issuer, SanType};
use time::{Duration, OffsetDateTime};
use tracing::debug;

use super::TpmSigningKey;

const TTL_SECS: i64 = 365 * 24 * 60 * 60;
const CACHE_TTL: u64 = TTL_SECS as u64 / 2;
const NOT_BEFORE_OFFSET: i64 = 60;

pub struct TpmAuthority {
    issuer: Issuer<'static, TpmSigningKey>,
    cache: Cache<Authority, Arc<ServerConfig>>,
    provider: Arc<CryptoProvider>,
}

impl TpmAuthority {
    pub fn new(
        issuer: Issuer<'static, TpmSigningKey>,
        cache_size: u64,
        provider: CryptoProvider,
    ) -> Self {
        Self {
            issuer,
            cache: Cache::builder()
                .max_capacity(cache_size)
                .time_to_live(std::time::Duration::from_secs(CACHE_TTL))
                .build(),
            provider: Arc::new(provider),
        }
    }

    /// One TPM/CNG signature: mint a fresh leaf key pair, ask the root to
    /// sign its certificate, return both the signed cert and the leaf's own
    /// (ordinary, exportable) private key.
    fn gen_leaf(&self, authority: &Authority) -> (CertificateDer<'static>, rcgen::KeyPair) {
        let leaf_key = rcgen::KeyPair::generate().expect("failed to generate leaf key pair");

        let mut params = CertificateParams::default();
        params.serial_number = Some(leaf_serial(authority.host()));

        let not_before = OffsetDateTime::now_utc() - Duration::seconds(NOT_BEFORE_OFFSET);
        params.not_before = not_before;
        params.not_after = not_before + Duration::seconds(TTL_SECS);

        let mut distinguished_name = DistinguishedName::new();
        distinguished_name.push(DnType::CommonName, authority.host());
        params.distinguished_name = distinguished_name;

        params.subject_alt_names.push(SanType::DnsName(
            rcgen::string::Ia5String::try_from(authority.host())
                .expect("host should be a valid DNS name"),
        ));

        let cert = params
            .signed_by(&leaf_key, &self.issuer)
            .expect("failed to sign leaf certificate through the TPM/CNG-backed issuer");

        (cert.der().clone(), leaf_key)
    }
}

impl CertificateAuthority for TpmAuthority {
    async fn gen_server_config(&self, authority: &Authority) -> Arc<ServerConfig> {
        if let Some(server_cfg) = self.cache.get(authority).await {
            debug!("Using cached server config");
            return server_cfg;
        }
        debug!("Generating server config (signing through CNG)");

        let (leaf_der, leaf_key) = self.gen_leaf(authority);
        let private_key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));

        let mut server_cfg = ServerConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_safe_default_protocol_versions()
            .expect("Failed to specify protocol versions")
            .with_no_client_auth()
            .with_single_cert(vec![leaf_der], private_key)
            .expect("Failed to build ServerConfig");

        server_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];

        let server_cfg = Arc::new(server_cfg);
        self.cache
            .insert(authority.clone(), Arc::clone(&server_cfg))
            .await;
        server_cfg
    }
}

/// A serial number that doesn't need a CSPRNG: leaf certs are locally
/// trusted, short-lived, and per-host, so this only needs to avoid
/// colliding with the previous cert for the *same* host within the cache's
/// TTL. Hashing the host with the current time is enough for that.
fn leaf_serial(host: &str) -> rcgen::SerialNumber {
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let material = format!("{host}-{nanos}");
    let digest = ring::digest::digest(&ring::digest::SHA256, material.as_bytes());
    rcgen::SerialNumber::from_slice(&digest.as_ref()[0..8])
}
