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
    /// Not secret: the legacy root's SHA-1 thumbprint, captured once, the
    /// first time a migration is detected, from whatever `CERT_FILE` holds
    /// at that moment. Every later retry reads the thumbprint back from
    /// here instead of re-deriving it from `CERT_FILE`, which a partially
    /// completed earlier attempt may already have overwritten with the new
    /// CNG-backed cert. Removed once migration finishes.
    pub const LEGACY_THUMBPRINT_FILE: &str = "blueflame-ca.legacy-thumbprint";
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
    let legacy_thumbprint_path = dir.join(LEGACY_THUMBPRINT_FILE);
    let fingerprint_path = dir.join(PUBKEY_FINGERPRINT_FILE);
    let backend_path = dir.join(BACKEND_FILE);
    let migrating = legacy_key_path.exists();

    // Capture the legacy root's thumbprint before anything below can
    // overwrite `cert_path` with the new CNG-backed cert, and only the
    // first time: a retry after an earlier, partially completed attempt
    // must reuse the recorded value rather than re-reading `cert_path`,
    // which that earlier attempt may already have overwritten with the
    // new cert (see `migrate_from_legacy_install` for why that matters).
    //
    // A failure here must abort this whole call rather than fall through:
    // `cert_is_stale` below is unconditionally true on a migrating install
    // with no fingerprint file yet, so falling through would overwrite
    // `cert_path` with the new cert before the legacy thumbprint was ever
    // durably recorded, destroying the only copy of the true legacy cert.
    // The next launch would then capture the *new* cert's thumbprint as if
    // it were the legacy one and have `migrate_from_legacy_install` remove
    // the only trusted BlueFlame root as though it were the old one.
    if migrating && !legacy_thumbprint_path.exists() {
        capture_legacy_thumbprint(&cert_path, &legacy_thumbprint_path).context(
            "capturing the legacy CA root's thumbprint before migrating off it; refusing to \
             touch the CA cert file until this is durably recorded",
        )?;
    }

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

    let cert_pem = if cert_is_stale {
        let pem = generate_cert_pem(&key)?;
        std::fs::write(&cert_path, &pem).context("writing CA cert")?;
        std::fs::write(&fingerprint_path, &fingerprint).context("writing CA key fingerprint")?;
        pem
    } else {
        std::fs::read_to_string(&cert_path).context("reading CA cert")?
    };

    if migrating {
        migrate_from_legacy_install(
            &cert_path,
            &cert_pem,
            &legacy_thumbprint_path,
            &legacy_key_path,
            trust,
        );
    }

    Ok(RootCa { cert_pem, key })
}

/// Read the legacy cert at `cert_path`, thumbprint it, and durably record
/// that thumbprint at `legacy_thumbprint_path` before the caller is allowed
/// to touch `cert_path` again. Every failure here (unreadable/corrupt cert,
/// unparseable PEM, a write that cannot land) must stop the caller cold:
/// there is no safe fallback that lets migration continue without a
/// recorded legacy thumbprint, since the very next step would otherwise
/// overwrite the only copy of the legacy cert.
#[cfg(target_os = "windows")]
fn capture_legacy_thumbprint(
    cert_path: &Path,
    legacy_thumbprint_path: &Path,
) -> anyhow::Result<()> {
    let legacy_pem = std::fs::read_to_string(cert_path).context(
        "legacy key file is present but its cert is not, so there is nothing safe to remove \
         from the trust store",
    )?;
    let thumb = crate::ca_trust::cert_thumbprint_sha1(&legacy_pem)
        .context("computing the legacy CA root's thumbprint")?;
    std::fs::write(legacy_thumbprint_path, &thumb)
        .context("recording the legacy CA root's thumbprint")?;
    Ok(())
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
/// so we can't touch some *other* cert that happens to share it, and never
/// by re-deriving it from `cert_path`, which may already hold the new cert
/// by the time this runs - see `legacy_thumbprint_path` in the caller), and
/// securely delete the old key file.
///
/// Every step here is idempotent and checks the trust store's actual state
/// rather than assuming what an earlier attempt did or didn't finish, so
/// this can be called any number of times, in any partially-completed
/// starting state (a crash or a failure can interrupt it after any step),
/// and always converges to: exactly one trusted BlueFlame root, matching
/// the CNG key actually in use, with the legacy key file gone only once
/// that is true.
///
/// If installing or verifying the new root fails, we deliberately leave
/// the old root trusted and the legacy key file in place, rather than
/// removing the old root or the old key anyway. Doing the removal/deletion
/// unconditionally would leave a machine trusting neither CA on that
/// failure, breaking every HTTPS site the proxy intercepts until someone
/// notices and re-runs the trust flow by hand. Leaving the legacy key file
/// behind also means `migrating` is still true on the next launch, so this
/// whole migration is retried automatically.
#[cfg(target_os = "windows")]
fn migrate_from_legacy_install(
    cert_path: &Path,
    cert_pem: &str,
    legacy_thumbprint_path: &Path,
    legacy_key_path: &Path,
    trust: &dyn crate::ca_trust::TrustOps,
) {
    tracing::info!("blueflame: migrating the CA root off a plaintext key file to CNG");

    let legacy_thumbprint = match std::fs::read_to_string(legacy_thumbprint_path) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(
                "blueflame: no recorded legacy CA root thumbprint yet, leaving the legacy key \
                 file in place; migration will retry on next launch: {e}"
            );
            return;
        }
    };
    let legacy_thumbprint = legacy_thumbprint.trim();

    // Installing an already-installed cert is a harmless no-op, so this is
    // always safe to (re)run rather than only on the very first attempt.
    if let Err(e) = trust.install(cert_path) {
        tracing::error!(
            "blueflame: could not auto-trust the migrated CA root, leaving the old root \
             trusted and the legacy key file in place so https interception keeps working; \
             migration will retry on next launch: {e}"
        );
        return;
    }

    let new_thumbprint = match crate::ca_trust::cert_thumbprint_sha1(cert_pem) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(
                "blueflame: could not compute the new CA root's thumbprint after installing \
                 it, leaving the old root and legacy key file in place; migration will retry \
                 on next launch: {e}"
            );
            return;
        }
    };
    if !trust.is_trusted(&new_thumbprint) {
        tracing::error!(
            "blueflame: the new CA root does not show up as trusted right after installing \
             it, leaving the old root and legacy key file in place; migration will retry on \
             next launch"
        );
        return;
    }

    // Only remove the old root if it is still there: an earlier attempt
    // may have already removed it and then been interrupted before it
    // could delete the legacy key file, and a real trust store errors out
    // on removing a thumbprint that is not present.
    if trust.is_trusted(legacy_thumbprint) {
        if let Err(e) = trust.remove_by_thumbprint(legacy_thumbprint) {
            tracing::warn!(
                "blueflame: could not remove the old CA root ({legacy_thumbprint}) from the \
                 trust store, leaving the legacy key file in place so migration retries the \
                 removal on next launch: {e}"
            );
            return;
        }
    }

    if let Err(e) = secure_delete(legacy_key_path) {
        tracing::warn!("blueflame: could not securely delete the legacy CA key file: {e}");
        return;
    }
    // Best-effort: leaving this behind after a successful migration would
    // not cause any wrong-cert removal (the legacy key file, the actual
    // trigger for migrating, is already gone by this point), only a
    // harmless leftover file.
    let _ = std::fs::remove_file(legacy_thumbprint_path);
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
    use crate::ca_trust::TrustOps;
    use rcgen::KeyPair;

    fn fake_store() -> Arc<dyn KeyStore> {
        Arc::new(ca_tpm::testing::FakeKeyStore::new())
    }

    /// Simulate a pre-TPM install: a self-signed cert + plaintext key file
    /// on disk, the state `migrate_from_legacy_install` needs to run.
    /// Returns the legacy cert's PEM.
    fn write_legacy_fixture(dir: &Path) -> String {
        std::fs::create_dir_all(dir).unwrap();
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
        legacy_cert.pem()
    }

    /// A fake trust store that already trusts the legacy root, as any real
    /// machine mid-migration would (that is the whole reason a migration
    /// runs at all).
    fn trust_with_legacy_seeded(legacy_pem: &str) -> FakeTrust {
        let legacy_thumb =
            crate::ca_trust::cert_thumbprint_sha1(legacy_pem).expect("legacy thumbprint");
        FakeTrust::seed_trusted(legacy_thumb)
    }

    /// Assert the fake trust store ends up trusting exactly one BlueFlame
    /// root, and that it is the TPM-backed cert actually in use.
    fn assert_only_new_root_trusted(trust: &FakeTrust, active_cert_pem: &str) {
        let new_thumb = crate::ca_trust::cert_thumbprint_sha1(active_cert_pem).expect("thumbprint");
        let trusted = trust.trusted.lock().unwrap();
        assert_eq!(
            trusted.len(),
            1,
            "expected exactly one trusted BlueFlame root, found {trusted:?}"
        );
        assert!(
            trusted.contains(&new_thumb),
            "the one trusted root should be the TPM-backed one actually in use"
        );
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
        let legacy_pem = write_legacy_fixture(dir);

        let store = fake_store();
        // A fake trust store: this test never runs `certutil` or touches
        // whatever real trust store this machine has.
        let trust = trust_with_legacy_seeded(&legacy_pem);
        let ca =
            load_or_create_with(dir, store, "blueflame-test-ca-migrate", &trust).expect("migrate");

        assert_ne!(
            ca.cert_pem, legacy_pem,
            "migration should mint a brand new cert, not reuse the legacy one"
        );
        assert!(
            !dir.join(windows_paths::LEGACY_KEY_FILE).exists(),
            "legacy key file should be deleted after migration"
        );
        assert!(
            !dir.join(windows_paths::LEGACY_THUMBPRINT_FILE).exists(),
            "the legacy thumbprint marker should be cleaned up once migration finishes"
        );

        assert_eq!(
            trust.install_calls.lock().unwrap().len(),
            1,
            "should auto-trust the new root exactly once"
        );
        assert_eq!(
            trust.removed.lock().unwrap().len(),
            1,
            "should remove exactly the old root by thumbprint"
        );
        assert_only_new_root_trusted(&trust, &ca.cert_pem);
    }

    #[test]
    fn install_fails_then_retry_succeeds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let legacy_pem = write_legacy_fixture(dir);

        let store = fake_store();
        let trust = trust_with_legacy_seeded(&legacy_pem);
        trust.set_fail_install(true);

        let first = load_or_create_with(
            dir,
            Arc::clone(&store),
            "blueflame-test-ca-retry-install",
            &trust,
        )
        .expect("load_or_create_with should not fail just because auto-trust failed");

        assert!(
            dir.join(windows_paths::LEGACY_KEY_FILE).exists(),
            "legacy key file must survive a failed install so migration retries next launch"
        );
        let legacy_thumb =
            crate::ca_trust::cert_thumbprint_sha1(&legacy_pem).expect("legacy thumbprint");
        assert!(
            trust.is_trusted(&legacy_thumb),
            "only the untouched legacy root should be trusted while install keeps failing"
        );
        assert_eq!(trust.trusted.lock().unwrap().len(), 1);

        // Retry: the install succeeds this time, as if a transient failure
        // (or a crash right after it) had cleared up.
        trust.set_fail_install(false);
        let second = load_or_create_with(dir, store, "blueflame-test-ca-retry-install", &trust)
            .expect("retry should succeed");

        assert_eq!(
            first.cert_pem, second.cert_pem,
            "retry should reuse the already-generated cert, not mint another one"
        );
        assert!(!dir.join(windows_paths::LEGACY_KEY_FILE).exists());
        assert!(!dir.join(windows_paths::LEGACY_THUMBPRINT_FILE).exists());
        assert_only_new_root_trusted(&trust, &second.cert_pem);
    }

    #[test]
    fn install_succeeds_but_removing_old_root_fails_then_retry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let legacy_pem = write_legacy_fixture(dir);

        let store = fake_store();
        let trust = trust_with_legacy_seeded(&legacy_pem);
        trust.set_fail_remove(true);

        let first = load_or_create_with(
            dir,
            Arc::clone(&store),
            "blueflame-test-ca-retry-remove",
            &trust,
        )
        .expect("load_or_create_with should not fail just because removing the old root failed");

        assert!(
            dir.join(windows_paths::LEGACY_KEY_FILE).exists(),
            "legacy key file must survive a failed removal so migration retries next launch"
        );
        assert_eq!(
            trust.trusted.lock().unwrap().len(),
            2,
            "both the new and the still-untouched old root should be trusted while removal keeps failing"
        );

        trust.set_fail_remove(false);
        let second = load_or_create_with(dir, store, "blueflame-test-ca-retry-remove", &trust)
            .expect("retry should succeed");

        assert_eq!(first.cert_pem, second.cert_pem);
        assert!(!dir.join(windows_paths::LEGACY_KEY_FILE).exists());
        assert!(!dir.join(windows_paths::LEGACY_THUMBPRINT_FILE).exists());
        assert_only_new_root_trusted(&trust, &second.cert_pem);
    }

    #[test]
    fn crash_after_install_before_verifying_trust_then_retry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let legacy_pem = write_legacy_fixture(dir);

        let store = fake_store();
        let trust = trust_with_legacy_seeded(&legacy_pem);
        trust.set_fail_is_trusted(true);

        load_or_create_with(
            dir,
            Arc::clone(&store),
            "blueflame-test-ca-retry-verify",
            &trust,
        )
        .expect("load_or_create_with should not fail just because verifying trust failed");

        assert!(
            dir.join(windows_paths::LEGACY_KEY_FILE).exists(),
            "legacy key file must survive a failed trust verification so migration retries \
             next launch"
        );
        assert!(
            trust.removed.lock().unwrap().is_empty(),
            "the old root must not be removed before the new one is confirmed trusted"
        );

        trust.set_fail_is_trusted(false);
        let ca = load_or_create_with(dir, store, "blueflame-test-ca-retry-verify", &trust)
            .expect("retry should succeed");

        assert!(!dir.join(windows_paths::LEGACY_KEY_FILE).exists());
        assert!(!dir.join(windows_paths::LEGACY_THUMBPRINT_FILE).exists());
        assert_only_new_root_trusted(&trust, &ca.cert_pem);
    }

    #[test]
    fn crash_after_capturing_legacy_thumbprint_then_retry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let legacy_pem = write_legacy_fixture(dir);

        // Simulate a crash right after the legacy thumbprint was recorded,
        // but before anything else in the migration ran.
        let legacy_thumb =
            crate::ca_trust::cert_thumbprint_sha1(&legacy_pem).expect("legacy thumbprint");
        std::fs::write(
            dir.join(windows_paths::LEGACY_THUMBPRINT_FILE),
            &legacy_thumb,
        )
        .unwrap();

        let store = fake_store();
        let trust = trust_with_legacy_seeded(&legacy_pem);
        let ca = load_or_create_with(dir, store, "blueflame-test-ca-crash-thumb", &trust)
            .expect("migrate after resuming from a recorded thumbprint");

        assert_ne!(ca.cert_pem, legacy_pem);
        assert!(!dir.join(windows_paths::LEGACY_KEY_FILE).exists());
        assert!(!dir.join(windows_paths::LEGACY_THUMBPRINT_FILE).exists());
        assert_only_new_root_trusted(&trust, &ca.cert_pem);
    }

    #[test]
    fn crash_after_removing_old_root_before_deleting_legacy_key_then_retry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let legacy_pem = write_legacy_fixture(dir);
        let legacy_thumb =
            crate::ca_trust::cert_thumbprint_sha1(&legacy_pem).expect("legacy thumbprint");

        // Get to the state a real run would be in right before it crashed:
        // the new CNG-backed cert already generated and installed, the old
        // root already removed, only the legacy key file (and its
        // thumbprint marker) still waiting to be deleted.
        let store = fake_store();
        let key =
            ca_tpm::load_or_create_root_key(Arc::clone(&store), "blueflame-test-ca-crash-remove")
                .expect("create cng key");
        let new_cert_pem = generate_cert_pem(&key).expect("generate new cert");
        std::fs::write(dir.join(CERT_FILE), &new_cert_pem).unwrap();
        std::fs::write(
            dir.join(windows_paths::PUBKEY_FINGERPRINT_FILE),
            hex_encode(&key.public_point()),
        )
        .unwrap();
        std::fs::write(
            dir.join(windows_paths::LEGACY_THUMBPRINT_FILE),
            &legacy_thumb,
        )
        .unwrap();

        let new_thumb = crate::ca_trust::cert_thumbprint_sha1(&new_cert_pem).expect("new thumb");
        let trust = FakeTrust::default();
        trust.trusted.lock().unwrap().insert(new_thumb);

        let ca = load_or_create_with(dir, store, "blueflame-test-ca-crash-remove", &trust)
            .expect("retry should finish cleanup");

        assert_eq!(
            ca.cert_pem, new_cert_pem,
            "retry should reuse the already-installed cert, not mint another one"
        );
        assert!(
            !dir.join(windows_paths::LEGACY_KEY_FILE).exists(),
            "legacy key file should finally be deleted"
        );
        assert!(!dir.join(windows_paths::LEGACY_THUMBPRINT_FILE).exists());
        assert!(
            trust.removed.lock().unwrap().is_empty(),
            "the old root was already gone before this call, so remove should not be called again"
        );
        assert_only_new_root_trusted(&trust, &ca.cert_pem);
    }

    #[test]
    fn thumbprint_capture_failure_aborts_without_touching_cert_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        std::fs::create_dir_all(dir).unwrap();

        // A legacy key file is present (so migration is detected) but the
        // cert file next to it is corrupt, not valid PEM at all.
        std::fs::write(dir.join(CERT_FILE), b"not a real cert").unwrap();
        std::fs::write(dir.join(windows_paths::LEGACY_KEY_FILE), b"not a real key").unwrap();

        let store = fake_store();
        let trust = FakeTrust::default();

        let result = load_or_create_with(dir, store, "blueflame-test-ca-capture-fail", &trust);
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!(
                "a capture failure must abort rather than proceed and overwrite the cert file"
            ),
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("thumbprint") || msg.contains("legacy"),
            "unexpected error: {msg}"
        );

        // Nothing migration will need later got touched.
        assert_eq!(
            std::fs::read(dir.join(CERT_FILE)).unwrap(),
            b"not a real cert",
            "the corrupt legacy cert file must be left exactly as it was, never overwritten"
        );
        assert!(
            dir.join(windows_paths::LEGACY_KEY_FILE).exists(),
            "the legacy key file must survive a capture failure"
        );
        assert!(
            !dir.join(windows_paths::LEGACY_THUMBPRINT_FILE).exists(),
            "no thumbprint should be recorded from a failed capture"
        );
        assert!(
            !dir.join(windows_paths::PUBKEY_FINGERPRINT_FILE).exists(),
            "no new cert/key should be generated when the capture step aborts first"
        );
        assert!(trust.trusted.lock().unwrap().is_empty());
    }

    #[test]
    fn retry_after_fixing_a_corrupt_legacy_cert_completes_migration() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        std::fs::create_dir_all(dir).unwrap();

        std::fs::write(dir.join(CERT_FILE), b"not a real cert").unwrap();
        std::fs::write(dir.join(windows_paths::LEGACY_KEY_FILE), b"not a real key").unwrap();

        let store = fake_store();
        let trust = FakeTrust::default();
        let first_attempt = load_or_create_with(
            dir,
            Arc::clone(&store),
            "blueflame-test-ca-capture-retry",
            &trust,
        );
        assert!(
            first_attempt.is_err(),
            "first attempt should abort on the corrupt cert"
        );

        // Whoever owns the box fixes the corrupt file and a real legacy
        // install shows up in its place.
        let legacy_pem = write_legacy_fixture(dir);
        let legacy_thumb =
            crate::ca_trust::cert_thumbprint_sha1(&legacy_pem).expect("legacy thumbprint");
        trust.trusted.lock().unwrap().insert(legacy_thumb);

        let ca = load_or_create_with(dir, store, "blueflame-test-ca-capture-retry", &trust)
            .expect("retry should succeed once the legacy cert is readable");

        assert!(!dir.join(windows_paths::LEGACY_KEY_FILE).exists());
        assert!(!dir.join(windows_paths::LEGACY_THUMBPRINT_FILE).exists());
        assert_only_new_root_trusted(&trust, &ca.cert_pem);
    }

    #[test]
    fn key_backend_defaults_to_unknown_without_a_marker_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(key_backend(tmp.path()), "unknown");
    }
}
