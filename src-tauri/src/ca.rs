//! Root CA cert management for the MITM proxy.
//!
//! On first run BlueFlame generates a self-signed root CA. The cert is
//! written to the user data dir so it can be installed into the OS trust
//! store.
//!
//! On Windows the private key never touches disk: it is generated inside
//! CNG (the TPM via the Microsoft Platform Crypto Provider when one is
//! present, otherwise the non-exportable Microsoft Software Key Storage
//! Provider) and every signature is produced by asking CNG to sign a hash.
//! See `ca_tpm.rs` for the CNG integration and exactly what that does and
//! does not protect against.
//!
//! On macOS/Linux/Android there is no equivalent OS-level non-exportable
//! key store wired up yet, so the key still lives in a PEM file next to the
//! cert, reloaded across restarts so the proxy doesn't regenerate the CA
//! (which would break any already-installed trust).

use std::path::{Path, PathBuf};

use anyhow::Context;
#[cfg(not(target_os = "windows"))]
use rcgen::KeyPair;
use rcgen::{CertificateParams, DistinguishedName, DnType, Issuer};

#[cfg(target_os = "windows")]
use crate::ca_tpm::{self, KeyStore};

const CA_COMMON_NAME: &str = "BlueFlame Root CA";
const CA_ORG: &str = "BlueFlame";
const CERT_FILE: &str = "blueflame-ca.crt";

#[cfg(not(target_os = "windows"))]
const KEY_FILE: &str = "blueflame-ca.key";

#[cfg(target_os = "windows")]
mod windows_paths {
    /// Pre-TPM installs kept the key here in plaintext; its presence is
    /// also how we detect "this install needs migrating".
    pub const LEGACY_KEY_FILE: &str = "blueflame-ca.key";
    /// Not secret: records which public key the current cert was issued
    /// for, so a restart can tell whether the CNG key backing it is still
    /// the same one without re-parsing the cert.
    pub const PUBKEY_FINGERPRINT_FILE: &str = "blueflame-ca.pubkey";
    /// Not secret: which CNG provider produced the key, surfaced to the UI.
    pub const BACKEND_FILE: &str = "blueflame-ca.backend";
    /// The CNG-persisted key name.
    pub const CNG_KEY_NAME: &str = "BlueFlameRootCA";
}

/// Loaded or freshly generated root CA usable by the MITM proxy.
pub struct RootCa {
    pub cert_pem: String,
    #[cfg(target_os = "windows")]
    pub key: ca_tpm::TpmSigningKey,
    #[cfg(not(target_os = "windows"))]
    pub key: KeyPair,
}

#[cfg(target_os = "windows")]
impl RootCa {
    /// Consume `self` and build a hudsucker-compatible `Issuer` backed by
    /// the CNG signing key. Used by `ca_tpm::TpmAuthority` to mint per-host
    /// leaf certs without ever exporting the root's private key.
    pub fn into_issuer(self) -> anyhow::Result<Issuer<'static, ca_tpm::TpmSigningKey>> {
        Issuer::from_ca_cert_pem(&self.cert_pem, self.key)
            .context("parsing CA cert PEM into Issuer")
    }
}

#[cfg(not(target_os = "windows"))]
impl RootCa {
    /// Consume `self` and build a hudsucker-compatible `Issuer`. The
    /// Issuer owns the parsed CA cert params + signing key, and is
    /// what hudsucker's `RcgenAuthority` uses to mint per-host leaf
    /// certs on the fly.
    pub fn into_issuer(self) -> anyhow::Result<Issuer<'static, KeyPair>> {
        Issuer::from_ca_cert_pem(&self.cert_pem, self.key)
            .context("parsing CA cert PEM into Issuer")
    }
}

/// Load the root CA from `dir`, or generate + persist a new one if missing.
#[cfg(not(target_os = "windows"))]
pub fn load_or_create<P: AsRef<Path>>(dir: P) -> anyhow::Result<RootCa> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).context("creating CA dir")?;

    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);

    if cert_path.exists() && key_path.exists() {
        let cert_pem = std::fs::read_to_string(&cert_path).context("reading CA cert")?;
        let key_pem = std::fs::read_to_string(&key_path).context("reading CA key")?;
        let key = KeyPair::from_pem(&key_pem).context("parsing CA key PEM")?;
        return Ok(RootCa { cert_pem, key });
    }

    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, CA_COMMON_NAME);
    params
        .distinguished_name
        .push(DnType::OrganizationName, CA_ORG);
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

    let key = KeyPair::generate().context("generating CA key pair")?;
    let cert = params.self_signed(&key).context("self-signing CA cert")?;
    let ca = RootCa {
        cert_pem: cert.pem(),
        key,
    };

    std::fs::write(&cert_path, &ca.cert_pem).context("writing CA cert")?;
    std::fs::write(&key_path, ca.key.serialize_pem()).context("writing CA key")?;
    Ok(ca)
}

/// Load the root CA from `dir`, or generate + persist a new one if missing.
/// The private key itself is never written to `dir` (or anywhere else on
/// disk); see `ca_tpm.rs`.
#[cfg(target_os = "windows")]
pub fn load_or_create<P: AsRef<Path>>(dir: P) -> anyhow::Result<RootCa> {
    load_or_create_with(
        dir,
        ca_tpm::default_store(),
        windows_paths::CNG_KEY_NAME,
        &crate::ca_trust::RealTrust,
    )
}

#[cfg(target_os = "windows")]
fn load_or_create_with<P: AsRef<Path>>(
    dir: P,
    store: std::sync::Arc<dyn KeyStore>,
    cng_key_name: &str,
    trust: &dyn crate::ca_trust::TrustOps,
) -> anyhow::Result<RootCa> {
    use windows_paths::*;

    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).context("creating CA dir")?;

    let cert_path = dir.join(CERT_FILE);
    let legacy_key_path = dir.join(LEGACY_KEY_FILE);
    let fingerprint_path = dir.join(PUBKEY_FINGERPRINT_FILE);
    let backend_path = dir.join(BACKEND_FILE);
    let migrating = legacy_key_path.exists();

    let key = ca_tpm::load_or_create_root_key(store, cng_key_name)
        .context("loading or creating the CNG-backed CA key")?;

    if key.backend() == ca_tpm::KeyBackend::Software {
        tracing::warn!(
            "blueflame: no TPM found on this machine - the CA root key is stored in \
             Windows' software key store instead (still non-exportable, but without \
             hardware backing)"
        );
    }
    let _ = std::fs::write(&backend_path, key.backend().as_str());

    let fingerprint = hex_encode(&key.public_point());
    let existing_fingerprint = std::fs::read_to_string(&fingerprint_path).ok();
    let cert_is_stale =
        !cert_path.exists() || existing_fingerprint.as_deref() != Some(fingerprint.as_str());

    // Read the outgoing cert (if any) before we potentially overwrite it,
    // so a legacy-key migration can remove exactly that cert (by
    // thumbprint) from the trust store below.
    let old_cert_pem = if migrating || cert_is_stale {
        std::fs::read_to_string(&cert_path).ok()
    } else {
        None
    };

    let cert_pem = if cert_is_stale {
        let pem = generate_cert_pem(&key)?;
        std::fs::write(&cert_path, &pem).context("writing CA cert")?;
        std::fs::write(&fingerprint_path, &fingerprint).context("writing CA key fingerprint")?;
        pem
    } else {
        std::fs::read_to_string(&cert_path).context("reading CA cert")?
    };

    if migrating {
        migrate_from_legacy_install(&cert_path, old_cert_pem.as_deref(), &legacy_key_path, trust);
    }

    Ok(RootCa { cert_pem, key })
}

#[cfg(target_os = "windows")]
fn generate_cert_pem(key: &ca_tpm::TpmSigningKey) -> anyhow::Result<String> {
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, CA_COMMON_NAME);
    params
        .distinguished_name
        .push(DnType::OrganizationName, CA_ORG);
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

    let cert = params
        .self_signed(key)
        .context("self-signing CA cert with the CNG-backed key")?;
    Ok(cert.pem())
}

/// Migrating off a pre-TPM install that kept the key in a plaintext file:
/// trust the new root the same way the normal first-run flow would, remove
/// the old root from the trust store by thumbprint (never by common name,
/// so we can't touch some *other* cert that happens to share it), and
/// securely delete the old key file.
///
/// The three steps are ordered on purpose and are not independent: if
/// installing the new root fails, we deliberately leave the old root
/// trusted and the legacy key file in place, rather than removing the
/// old root or the old key anyway. Doing the removal/deletion
/// unconditionally would leave a machine trusting neither CA on that
/// failure, breaking every HTTPS site the proxy intercepts until someone
/// notices and re-runs the trust flow by hand. Leaving the legacy key
/// file behind also means `migrating` is still true on the next launch,
/// so this whole migration is retried automatically.
#[cfg(target_os = "windows")]
fn migrate_from_legacy_install(
    cert_path: &Path,
    old_cert_pem: Option<&str>,
    legacy_key_path: &Path,
    trust: &dyn crate::ca_trust::TrustOps,
) {
    tracing::info!("blueflame: migrating the CA root off a plaintext key file to CNG");

    if let Err(e) = trust.install(cert_path) {
        tracing::error!(
            "blueflame: could not auto-trust the migrated CA root, leaving the old root \
             trusted and the legacy key file in place so https interception keeps working; \
             migration will retry on next launch: {e}"
        );
        return;
    }

    if let Some(old_pem) = old_cert_pem {
        match thumbprint_sha1(old_pem) {
            Ok(thumb) => {
                if let Err(e) = trust.remove_by_thumbprint(&thumb) {
                    tracing::warn!(
                        "blueflame: could not remove the old CA root ({thumb}) from the trust store: {e}"
                    );
                }
            }
            Err(e) => {
                tracing::warn!("blueflame: could not compute the old CA root's thumbprint: {e}")
            }
        }
    }

    if let Err(e) = secure_delete(legacy_key_path) {
        tracing::warn!("blueflame: could not securely delete the legacy CA key file: {e}");
    }
}

/// Best-effort secure delete: overwrite with zeros before unlinking. This
/// does not guarantee the bytes are gone from wear-levelled SSD flash, but
/// it beats a plain delete against casual recovery, and once this returns
/// there is no path left on disk that ever holds the key in the clear.
#[cfg(target_os = "windows")]
fn secure_delete(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let len = std::fs::metadata(path)
        .context("stat legacy key file")?
        .len();
    std::fs::write(path, vec![0u8; len as usize]).context("zeroing legacy key file")?;
    std::fs::remove_file(path).context("removing legacy key file")?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn thumbprint_sha1(cert_pem: &str) -> anyhow::Result<String> {
    let parsed = pem::parse(cert_pem).context("parsing PEM")?;
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, parsed.contents());
    Ok(digest.as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(target_os = "windows")]
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Which key backend produced the currently active CA key. Best-effort,
/// read from the marker file `load_or_create` writes; used only for UI/log
/// display, never for trust decisions. Non-Windows installs always report
/// the plain PEM file.
pub fn key_backend<P: AsRef<Path>>(dir: P) -> String {
    #[cfg(target_os = "windows")]
    {
        std::fs::read_to_string(dir.as_ref().join(windows_paths::BACKEND_FILE))
            .unwrap_or_else(|_| "unknown".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = dir;
        "file".to_string()
    }
}

/// Path to the cert file the user needs to trust in their OS.
#[allow(dead_code)] // exposed for a future "reveal in explorer" command
pub fn cert_path(dir: &Path) -> PathBuf {
    dir.join(CERT_FILE)
}

#[cfg(not(target_os = "windows"))]
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
    fn cert_pem_parses_as_issuer() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ca = load_or_create(tmp.path()).expect("generate");
        // If the PEM is a valid CA cert that matches the saved key pair,
        // Issuer::from_ca_cert_pem accepts both. That's the only proof the
        // proxy needs that the round-trip works.
        ca.into_issuer().expect("parse PEM as Issuer");
    }
}

#[cfg(target_os = "windows")]
#[cfg(test)]
mod windows_tests {
    use super::*;
    use std::sync::Arc;

    use crate::ca_trust::testing::FakeTrust;
    use rcgen::KeyPair;

    fn fake_store() -> Arc<dyn KeyStore> {
        Arc::new(ca_tpm::testing::FakeKeyStore::new())
    }

    #[test]
    fn generates_and_reloads_same_ca_with_no_key_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = fake_store();
        let trust = FakeTrust::default();

        let first = load_or_create_with(
            tmp.path(),
            Arc::clone(&store),
            "blueflame-test-ca-1",
            &trust,
        )
        .expect("generate");
        let second = load_or_create_with(
            tmp.path(),
            Arc::clone(&store),
            "blueflame-test-ca-1",
            &trust,
        )
        .expect("reload");
        assert_eq!(first.cert_pem, second.cert_pem, "cert should be stable");

        // The whole point: no key file anywhere in the CA dir.
        for entry in std::fs::read_dir(tmp.path()).expect("read ca dir") {
            let entry = entry.expect("dir entry");
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.ends_with(".key"),
                "found a plaintext key file after a TPM-backed load: {name}"
            );
        }
    }

    #[test]
    fn cert_pem_parses_as_issuer() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = fake_store();
        let trust = FakeTrust::default();
        let ca = load_or_create_with(tmp.path(), store, "blueflame-test-ca-2", &trust)
            .expect("generate");
        ca.into_issuer().expect("parse PEM as Issuer");
    }

    #[test]
    fn migrates_legacy_key_file_and_deletes_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        std::fs::create_dir_all(dir).unwrap();

        // Simulate a pre-TPM install: a self-signed cert + plaintext key.
        let legacy_key = KeyPair::generate().expect("legacy key");
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, CA_COMMON_NAME);
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let legacy_cert = params.self_signed(&legacy_key).expect("self sign legacy");

        std::fs::write(dir.join(CERT_FILE), legacy_cert.pem()).unwrap();
        std::fs::write(
            dir.join(windows_paths::LEGACY_KEY_FILE),
            legacy_key.serialize_pem(),
        )
        .unwrap();

        let store = fake_store();
        // A fake trust store: this test never runs `certutil` or touches
        // whatever real trust store this machine has.
        let trust = FakeTrust::default();
        let ca =
            load_or_create_with(dir, store, "blueflame-test-ca-migrate", &trust).expect("migrate");

        assert_ne!(
            ca.cert_pem,
            legacy_cert.pem(),
            "migration should mint a brand new cert, not reuse the legacy one"
        );
        assert!(
            !dir.join(windows_paths::LEGACY_KEY_FILE).exists(),
            "legacy key file should be deleted after migration"
        );

        let installed = trust.installed.lock().unwrap();
        assert_eq!(
            installed.len(),
            1,
            "should auto-trust the new root exactly once"
        );
        assert_eq!(installed[0], dir.join(CERT_FILE));

        let removed = trust.removed.lock().unwrap();
        assert_eq!(
            removed.len(),
            1,
            "should remove exactly the old root by thumbprint"
        );
    }

    #[test]
    fn keeps_old_root_and_legacy_key_when_installing_the_new_root_fails() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        std::fs::create_dir_all(dir).unwrap();

        // Same pre-TPM install fixture as the happy-path test above.
        let legacy_key = KeyPair::generate().expect("legacy key");
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, CA_COMMON_NAME);
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let legacy_cert = params.self_signed(&legacy_key).expect("self sign legacy");

        std::fs::write(dir.join(CERT_FILE), legacy_cert.pem()).unwrap();
        std::fs::write(
            dir.join(windows_paths::LEGACY_KEY_FILE),
            legacy_key.serialize_pem(),
        )
        .unwrap();

        let store = fake_store();
        let trust = FakeTrust {
            fail_install: true,
            ..Default::default()
        };
        load_or_create_with(dir, store, "blueflame-test-ca-migrate-fail", &trust)
            .expect("load_or_create_with should not fail just because auto-trust failed");

        assert!(
            dir.join(windows_paths::LEGACY_KEY_FILE).exists(),
            "legacy key file must survive a failed install so migration retries next launch"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(windows_paths::LEGACY_KEY_FILE)).unwrap(),
            legacy_key.serialize_pem(),
            "legacy key file must not be zeroed or touched when install fails"
        );
        assert!(
            trust.removed.lock().unwrap().is_empty(),
            "the old root must not be removed from the trust store when installing the new root failed, \
             or the machine would end up trusting neither CA"
        );
    }

    #[test]
    fn key_backend_defaults_to_unknown_without_a_marker_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(key_backend(tmp.path()), "unknown");
    }
}
