//! Windows-only: the root CA private key lives inside CNG (Cryptography API:
//! Next Generation) instead of a plaintext PEM file.
//!
//! `NCryptCreatePersistedKey` on the "Microsoft Platform Crypto Provider"
//! asks the TPM to generate and hold a non-exportable EC P-256 key. The key
//! material never crosses into user-mode memory: every signature is produced
//! by asking the TPM to sign a hash, and `NCRYPT_EXPORT_POLICY_PROPERTY` is
//! set to disallow export explicitly (on top of it already being the
//! default for a freshly created key). If no TPM is present, or the
//! platform provider otherwise refuses to create a key, we fall back to the
//! "Microsoft Software Key Storage Provider" with the same non-exportable
//! policy - still no plaintext key file, just no hardware binding.
//!
//! The CNG access itself is behind the [`KeyStore`] trait so tests can fake
//! it; only `real_cng_tests` below (gated to Windows) exercises the actual
//! provider, and it always uses a `blueflame-test-` key name that it deletes
//! when it's done.
//!
//! What this protects against: a copy of BlueFlame's app-data directory, or
//! of the whole disk, no longer hands over a working root CA key. What it
//! does NOT protect against: malware already running as the same Windows
//! user can still ask the TPM/CNG to sign arbitrary leaf certificates for as
//! long as it runs (the key is usable by anyone who can act as this user),
//! it just cannot copy the key out to use elsewhere or after the fact.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context};
use rcgen::{PublicKeyData, SignatureAlgorithm, SigningKey};
use windows_sys::Win32::Security::Cryptography::{
    NCryptCreatePersistedKey, NCryptDeleteKey, NCryptExportKey, NCryptFinalizeKey,
    NCryptFreeObject, NCryptOpenKey, NCryptOpenStorageProvider, NCryptSetProperty, NCryptSignHash,
    BCRYPT_ECCPUBLIC_BLOB, BCRYPT_ECDSA_P256_ALGORITHM, BCRYPT_ECDSA_PUBLIC_P256_MAGIC,
    MS_KEY_STORAGE_PROVIDER, MS_PLATFORM_CRYPTO_PROVIDER, NCRYPT_EXPORT_POLICY_PROPERTY,
    NCRYPT_KEY_HANDLE, NCRYPT_PROV_HANDLE, NCRYPT_SILENT_FLAG,
};

mod authority;
pub use authority::TpmAuthority;

/// Which CNG provider actually holds the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyBackend {
    /// Microsoft Platform Crypto Provider - key generated and held by the TPM.
    Tpm,
    /// Microsoft Software Key Storage Provider - no TPM was available.
    Software,
}

impl KeyBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            KeyBackend::Tpm => "tpm",
            KeyBackend::Software => "software",
        }
    }
}

/// A persisted CNG key: the raw handles plus the public key we read back
/// once at create/open time. Handles are plain OS handles (not RAII) - see
/// the note on [`KeyStore::delete`] for why.
#[derive(Debug, Clone, Copy)]
pub struct KeyRecord {
    // Only read back by `KeyStore::delete`, which the running app never
    // calls on its own root key (see the note there) - only tests and any
    // future cleanup tooling do.
    #[allow(dead_code)]
    provider: NCRYPT_PROV_HANDLE,
    key: NCRYPT_KEY_HANDLE,
    pub point: [u8; 65],
    pub backend: KeyBackend,
}

/// Abstraction over "a place that can create/open/use/delete a named,
/// non-exportable EC P-256 key" so the CA logic (and its tests) never have
/// to talk to real CNG directly. `CngKeyStore` is the real Windows
/// implementation; `testing::FakeKeyStore` is an in-memory stand-in used by
/// every test that isn't specifically about the CNG FFI itself.
pub trait KeyStore: Send + Sync {
    /// Create a new persisted key named `name`. Tries the TPM-backed
    /// platform provider first and falls back to the software provider.
    fn create(&self, name: &str) -> anyhow::Result<KeyRecord>;
    /// Open an existing persisted key named `name`, if one exists in either
    /// provider.
    fn open(&self, name: &str) -> anyhow::Result<Option<KeyRecord>>;
    /// Sign a precomputed SHA-256 hash, returning the raw (r || s) ECDSA
    /// signature (64 bytes for P-256).
    fn sign(&self, record: &KeyRecord, hash32: &[u8; 32]) -> anyhow::Result<[u8; 64]>;
    /// Permanently delete a persisted key from whichever provider holds it.
    /// The running app never deletes its own root key; this exists for
    /// tests (which always clean up whatever they create) and any future
    /// "remove BlueFlame's CA" cleanup tooling.
    #[allow(dead_code)]
    fn delete(&self, record: KeyRecord) -> anyhow::Result<()>;
}

/// The real CNG-backed store.
pub struct CngKeyStore {
    // NCrypt key handles are not documented as safe for concurrent use from
    // multiple threads; the proxy only signs once per newly seen host
    // (results are cached, see `authority.rs`) so this is never hot.
    sign_lock: Mutex<()>,
}

impl CngKeyStore {
    pub fn new() -> Self {
        Self {
            sign_lock: Mutex::new(()),
        }
    }
}

impl Default for CngKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore for CngKeyStore {
    fn create(&self, name: &str) -> anyhow::Result<KeyRecord> {
        match create_on_provider(MS_PLATFORM_CRYPTO_PROVIDER, name) {
            Ok((provider, key, point)) => Ok(KeyRecord {
                provider,
                key,
                point,
                backend: KeyBackend::Tpm,
            }),
            Err(tpm_err) => {
                tracing::warn!(
                    "blueflame: could not create the CA key in the TPM ({tpm_err}), \
                     falling back to the software key storage provider"
                );
                let (provider, key, point) = create_on_provider(MS_KEY_STORAGE_PROVIDER, name)
                    .context("creating the CA key in the software key storage provider")?;
                Ok(KeyRecord {
                    provider,
                    key,
                    point,
                    backend: KeyBackend::Software,
                })
            }
        }
    }

    fn open(&self, name: &str) -> anyhow::Result<Option<KeyRecord>> {
        for (provider_name, backend) in [
            (MS_PLATFORM_CRYPTO_PROVIDER, KeyBackend::Tpm),
            (MS_KEY_STORAGE_PROVIDER, KeyBackend::Software),
        ] {
            let mut hprov: NCRYPT_PROV_HANDLE = 0;
            // SAFETY: `phprovider` and `pszprovidername` both point at valid
            // memory for the duration of the call (a local and a static
            // wide-string literal respectively).
            let hr = unsafe { NCryptOpenStorageProvider(&mut hprov, provider_name, 0) };
            if hr != 0 {
                continue;
            }

            let name_w = wide(name);
            let mut hkey: NCRYPT_KEY_HANDLE = 0;
            // SAFETY: `hprov` was just opened above, `phkey` and
            // `pszkeyname` point at valid memory for the call.
            let hr =
                unsafe { NCryptOpenKey(hprov, &mut hkey, name_w.as_ptr(), 0, NCRYPT_SILENT_FLAG) };
            if hr != 0 {
                unsafe { NCryptFreeObject(hprov) };
                continue;
            }

            match export_public_point(hkey) {
                Ok(point) => {
                    return Ok(Some(KeyRecord {
                        provider: hprov,
                        key: hkey,
                        point,
                        backend,
                    }))
                }
                Err(e) => {
                    tracing::warn!(
                        "blueflame: found a persisted CA key but could not read its public key: {e}"
                    );
                    unsafe {
                        NCryptFreeObject(hkey);
                        NCryptFreeObject(hprov);
                    }
                    continue;
                }
            }
        }
        Ok(None)
    }

    fn sign(&self, record: &KeyRecord, hash32: &[u8; 32]) -> anyhow::Result<[u8; 64]> {
        let _guard = self.sign_lock.lock().unwrap_or_else(|e| e.into_inner());
        sign_hash(record.key, hash32)
    }

    // NCryptDeleteKey also frees the key handle (per the Windows SDK docs,
    // callers must not call NCryptFreeObject on it afterwards), so this
    // takes `record` by value instead of exposing a Drop impl on
    // `KeyRecord` that could race with it.
    fn delete(&self, record: KeyRecord) -> anyhow::Result<()> {
        let hr = unsafe { NCryptDeleteKey(record.key, 0) };
        unsafe { NCryptFreeObject(record.provider) };
        if hr != 0 {
            bail!("NCryptDeleteKey failed: {}", hresult_str(hr));
        }
        Ok(())
    }
}

fn create_on_provider(
    provider_name: windows_sys::core::PCWSTR,
    key_name: &str,
) -> anyhow::Result<(NCRYPT_PROV_HANDLE, NCRYPT_KEY_HANDLE, [u8; 65])> {
    let mut hprov: NCRYPT_PROV_HANDLE = 0;
    // SAFETY: `phprovider` points at a local, `pszprovidername` is a
    // 'static wide-string literal from windows-sys.
    let hr = unsafe { NCryptOpenStorageProvider(&mut hprov, provider_name, 0) };
    if hr != 0 {
        bail!("NCryptOpenStorageProvider failed: {}", hresult_str(hr));
    }

    let name_w = wide(key_name);
    let mut hkey: NCRYPT_KEY_HANDLE = 0;
    // SAFETY: `hprov` was just opened, `phkey`/`pszalgid`/`pszkeyname`
    // point at valid memory for the call.
    let hr = unsafe {
        NCryptCreatePersistedKey(
            hprov,
            &mut hkey,
            BCRYPT_ECDSA_P256_ALGORITHM,
            name_w.as_ptr(),
            0,
            0,
        )
    };
    if hr != 0 {
        unsafe { NCryptFreeObject(hprov) };
        bail!("NCryptCreatePersistedKey failed: {}", hresult_str(hr));
    }

    // Explicit non-exportable export policy. This is already the default
    // for a freshly created key with no policy set, but we set it
    // explicitly so the intent is in the code, not just relied on as a
    // default that some future CNG change could quietly alter.
    let export_policy: u32 = 0;
    // SAFETY: `hkey` was just created, `pbinput` points at a local `u32`
    // that outlives the call.
    let hr = unsafe {
        NCryptSetProperty(
            hkey,
            NCRYPT_EXPORT_POLICY_PROPERTY,
            &export_policy as *const u32 as *const u8,
            std::mem::size_of::<u32>() as u32,
            0,
        )
    };
    if hr != 0 {
        unsafe {
            NCryptFreeObject(hkey);
            NCryptFreeObject(hprov);
        }
        bail!(
            "NCryptSetProperty(Export Policy) failed: {}",
            hresult_str(hr)
        );
    }

    // SAFETY: `hkey` is a valid, not-yet-finalized key handle.
    let hr = unsafe { NCryptFinalizeKey(hkey, NCRYPT_SILENT_FLAG) };
    if hr != 0 {
        unsafe {
            // The key was created but never finalized; delete the partial
            // key rather than leaving an unusable stub behind.
            NCryptDeleteKey(hkey, 0);
            NCryptFreeObject(hprov);
        }
        bail!("NCryptFinalizeKey failed: {}", hresult_str(hr));
    }

    let point = match export_public_point(hkey) {
        Ok(p) => p,
        Err(e) => {
            unsafe {
                NCryptDeleteKey(hkey, 0);
                NCryptFreeObject(hprov);
            }
            return Err(e);
        }
    };

    Ok((hprov, hkey, point))
}

/// Read back the public key as an uncompressed EC point (0x04 || X || Y),
/// the same format `rcgen`/`ring` use for an in-memory P-256 `KeyPair`.
fn export_public_point(key: NCRYPT_KEY_HANDLE) -> anyhow::Result<[u8; 65]> {
    let mut needed: u32 = 0;
    // SAFETY: querying the required size; output buffer pointer is null as
    // the API allows for a size query.
    let hr = unsafe {
        NCryptExportKey(
            key,
            0,
            BCRYPT_ECCPUBLIC_BLOB,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
            &mut needed,
            0,
        )
    };
    if hr != 0 {
        bail!("NCryptExportKey (size query) failed: {}", hresult_str(hr));
    }

    let mut buf = vec![0u8; needed as usize];
    let mut written: u32 = 0;
    // SAFETY: `buf` is sized to `needed` bytes as returned by the query
    // above.
    let hr = unsafe {
        NCryptExportKey(
            key,
            0,
            BCRYPT_ECCPUBLIC_BLOB,
            std::ptr::null(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut written,
            0,
        )
    };
    if hr != 0 {
        bail!("NCryptExportKey failed: {}", hresult_str(hr));
    }
    buf.truncate(written as usize);

    // BCRYPT_ECCKEY_BLOB { dwMagic: u32, cbKey: u32 } followed by X then Y,
    // each cbKey bytes (big-endian magnitude, no sign byte).
    if buf.len() < 8 {
        bail!("ECC public key blob too short ({} bytes)", buf.len());
    }
    let magic = u32::from_ne_bytes(buf[0..4].try_into().unwrap());
    let cb_key = u32::from_ne_bytes(buf[4..8].try_into().unwrap()) as usize;
    if magic != BCRYPT_ECDSA_PUBLIC_P256_MAGIC {
        bail!("unexpected ECC key magic 0x{magic:08x} (expected P-256)");
    }
    if cb_key != 32 {
        bail!("unexpected P-256 field size {cb_key}");
    }
    if buf.len() < 8 + cb_key * 2 {
        bail!("ECC public key blob truncated");
    }

    let mut point = [0u8; 65];
    point[0] = 0x04;
    point[1..33].copy_from_slice(&buf[8..40]);
    point[33..65].copy_from_slice(&buf[40..72]);
    Ok(point)
}

fn sign_hash(key: NCRYPT_KEY_HANDLE, hash: &[u8; 32]) -> anyhow::Result<[u8; 64]> {
    let mut sig = [0u8; 64];
    let mut written: u32 = 0;
    // SAFETY: `hash` and `sig` are both fixed-size local arrays; `key` is a
    // valid, open ECDSA key handle. ECDSA ignores `pPaddingInfo`.
    let hr = unsafe {
        NCryptSignHash(
            key,
            std::ptr::null::<c_void>(),
            hash.as_ptr(),
            hash.len() as u32,
            sig.as_mut_ptr(),
            sig.len() as u32,
            &mut written,
            NCRYPT_SILENT_FLAG,
        )
    };
    if hr != 0 {
        bail!("NCryptSignHash failed: {}", hresult_str(hr));
    }
    if written as usize != sig.len() {
        bail!("unexpected ECDSA signature length {written}");
    }
    Ok(sig)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn hresult_str(hr: i32) -> String {
    format!("0x{:08x}", hr as u32)
}

/// A [`SigningKey`]/[`PublicKeyData`] that signs through a [`KeyStore`],
/// used as `rcgen`'s external signer for the root CA. The private key
/// itself is never in this struct, or anywhere else in this process's
/// memory: every `sign()` call round-trips through CNG.
pub struct TpmSigningKey {
    store: Arc<dyn KeyStore>,
    record: KeyRecord,
}

impl TpmSigningKey {
    pub fn backend(&self) -> KeyBackend {
        self.record.backend
    }

    pub fn public_point(&self) -> [u8; 65] {
        self.record.point
    }
}

impl PublicKeyData for TpmSigningKey {
    fn der_bytes(&self) -> &[u8] {
        &self.record.point
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &rcgen::PKCS_ECDSA_P256_SHA256
    }
}

impl SigningKey for TpmSigningKey {
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        let hash = ring::digest::digest(&ring::digest::SHA256, msg);
        let hash32: [u8; 32] = hash
            .as_ref()
            .try_into()
            .map_err(|_| rcgen::Error::RemoteKeyError)?;
        let raw = self
            .store
            .sign(&self.record, &hash32)
            .map_err(|_| rcgen::Error::RemoteKeyError)?;
        Ok(der_encode_ecdsa_sig(&raw))
    }
}

/// Load the persisted root CA key named `name` from `store`, creating it if
/// it doesn't exist yet.
pub fn load_or_create_root_key(
    store: Arc<dyn KeyStore>,
    name: &str,
) -> anyhow::Result<TpmSigningKey> {
    let record = match store.open(name)? {
        Some(r) => r,
        None => store.create(name)?,
    };
    Ok(TpmSigningKey { store, record })
}

/// The production key store: real CNG, TPM-backed when possible.
pub fn default_store() -> Arc<dyn KeyStore> {
    Arc::new(CngKeyStore::new())
}

/// Encode a raw (r || s) ECDSA signature (each half big-endian, zero-padded
/// to the field size) as the ASN.1 DER `SEQUENCE { INTEGER r, INTEGER s }`
/// that X.509 (and `rcgen`'s `PKCS_ECDSA_P256_SHA256`) expects.
fn der_encode_ecdsa_sig(raw: &[u8; 64]) -> Vec<u8> {
    let r = der_uint(&raw[0..32]);
    let s = der_uint(&raw[32..64]);
    let mut body = Vec::with_capacity(r.len() + s.len());
    body.extend_from_slice(&r);
    body.extend_from_slice(&s);

    let mut out = Vec::with_capacity(body.len() + 4);
    out.push(0x30); // SEQUENCE
    push_der_len(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

fn der_uint(magnitude: &[u8]) -> Vec<u8> {
    let mut b = magnitude;
    while b.len() > 1 && b[0] == 0 {
        b = &b[1..];
    }
    let mut content = b.to_vec();
    if content.is_empty() {
        content.push(0);
    }
    if content[0] & 0x80 != 0 {
        content.insert(0, 0x00);
    }
    let mut out = Vec::with_capacity(content.len() + 2);
    out.push(0x02); // INTEGER
    push_der_len(&mut out, content.len());
    out.extend_from_slice(&content);
    out
}

fn push_der_len(out: &mut Vec<u8>, len: usize) {
    if len < 128 {
        out.push(len as u8);
        return;
    }
    let mut len_bytes = Vec::new();
    let mut l = len;
    while l > 0 {
        len_bytes.insert(0, (l & 0xff) as u8);
        l >>= 8;
    }
    out.push(0x80 | len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
}

/// An in-memory fake of [`KeyStore`] for tests that need CA/key plumbing to
/// behave correctly without ever calling into real CNG. Not compiled into
/// the production binary.
#[cfg(test)]
pub mod testing {
    use super::*;
    use std::collections::HashMap;

    struct FakeKey {
        key_pair: ring::signature::EcdsaKeyPair,
        point: [u8; 65],
    }

    /// In-memory stand-in for [`CngKeyStore`]. Every "persisted" key is an
    /// ordinary in-process software ECDSA P-256 key pair; nothing here ever
    /// touches the real CNG store, the TPM, or the disk.
    #[derive(Default)]
    pub struct FakeKeyStore {
        keys: Mutex<HashMap<String, Arc<FakeKey>>>,
    }

    impl FakeKeyStore {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn contains(&self, name: &str) -> bool {
            self.keys.lock().unwrap().contains_key(name)
        }
    }

    impl KeyStore for FakeKeyStore {
        fn create(&self, name: &str) -> anyhow::Result<KeyRecord> {
            let rng = ring::rand::SystemRandom::new();
            let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(
                &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                &rng,
            )
            .map_err(|e| anyhow::anyhow!("fake key generation failed: {e}"))?;
            let key_pair = ring::signature::EcdsaKeyPair::from_pkcs8(
                &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                pkcs8.as_ref(),
                &rng,
            )
            .map_err(|e| anyhow::anyhow!("fake key load failed: {e}"))?;

            let mut point = [0u8; 65];
            point.copy_from_slice(ring::signature::KeyPair::public_key(&key_pair).as_ref());

            self.keys
                .lock()
                .unwrap()
                .insert(name.to_string(), Arc::new(FakeKey { key_pair, point }));

            Ok(KeyRecord {
                provider: 0,
                key: 0,
                point,
                backend: KeyBackend::Tpm,
            })
        }

        fn open(&self, name: &str) -> anyhow::Result<Option<KeyRecord>> {
            Ok(self.keys.lock().unwrap().get(name).map(|k| KeyRecord {
                provider: 0,
                key: 0,
                point: k.point,
                backend: KeyBackend::Tpm,
            }))
        }

        fn sign(&self, record: &KeyRecord, hash32: &[u8; 32]) -> anyhow::Result<[u8; 64]> {
            let keys = self.keys.lock().unwrap();
            let fake = keys
                .values()
                .find(|k| k.point == record.point)
                .ok_or_else(|| anyhow::anyhow!("fake key not found for record"))?;
            // The fixed-signing algorithm returns a raw (r || s) signature
            // directly, over the already-hashed input treated as the
            // message (ring re-hashes; since our input is a hash, not the
            // original message, this only needs to be self-consistent for
            // tests, not bit-for-bit identical to CNG's raw-hash signing).
            let rng = ring::rand::SystemRandom::new();
            let sig = fake
                .key_pair
                .sign(&rng, hash32)
                .map_err(|e| anyhow::anyhow!("fake sign failed: {e}"))?;
            let mut out = [0u8; 64];
            out.copy_from_slice(sig.as_ref());
            Ok(out)
        }

        fn delete(&self, record: KeyRecord) -> anyhow::Result<()> {
            let mut keys = self.keys.lock().unwrap();
            let name = keys
                .iter()
                .find(|(_, k)| k.point == record.point)
                .map(|(n, _)| n.clone());
            if let Some(name) = name {
                keys.remove(&name);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn der_encode_strips_leading_zeros_and_pads_high_bit() {
        let mut raw = [0u8; 64];
        // r is all leading zero bytes except the last two, which should
        // strip down to a two-byte content.
        raw[30] = 0x01;
        raw[31] = 0x02;
        // s has its top bit set in its only nonzero byte, so DER must
        // prepend a 0x00 even after leading-zero stripping.
        raw[63] = 0x80;

        let der = der_encode_ecdsa_sig(&raw);
        assert_eq!(der[0], 0x30, "outer tag should be SEQUENCE");
        assert_eq!(der[2], 0x02, "first element should be INTEGER (r)");
        // r = 0x01 0x02 with the leading zeros stripped down to two
        // content bytes, no extra 0x00 padding needed since the top bit of
        // 0x01 is not set.
        let r_len = der[3] as usize;
        assert_eq!(r_len, 2);
        assert_eq!(&der[4..4 + r_len], &[0x01, 0x02]);
        let s_tag_at = 4 + r_len;
        assert_eq!(der[s_tag_at], 0x02, "second element should be INTEGER (s)");
        let s_len = der[s_tag_at + 1] as usize;
        // s's high bit is set, so DER must have inserted a leading 0x00.
        assert_eq!(der[s_tag_at + 2], 0x00);
        assert_eq!(s_len, 2);
    }

    #[test]
    fn der_encode_handles_all_zero_component() {
        let raw = [0u8; 64];
        let der = der_encode_ecdsa_sig(&raw);
        // r and s both collapse to a single 0x00 content byte.
        assert_eq!(&der[0..5], &[0x30, 6, 0x02, 1, 0x00]);
    }

    #[test]
    fn fake_store_round_trips_create_open_sign_delete() {
        let store = testing::FakeKeyStore::new();
        let name = "blueflame-test-fake";

        let created = store.create(name).expect("create");
        let reopened = store.open(name).expect("open").expect("should exist");
        assert_eq!(created.point, reopened.point);

        let hash = [7u8; 32];
        let sig = store.sign(&reopened, &hash).expect("sign");
        assert_eq!(sig.len(), 64);

        store.delete(reopened).expect("delete");
        assert!(store.open(name).expect("open after delete").is_none());
        assert!(!store.contains(name));
    }

    #[test]
    fn signing_key_reports_backend_and_point() {
        let store: Arc<dyn KeyStore> = Arc::new(testing::FakeKeyStore::new());
        let key = load_or_create_root_key(Arc::clone(&store), "blueflame-test-signing-key")
            .expect("load or create");
        assert_eq!(key.backend(), KeyBackend::Tpm);
        assert_eq!(key.public_point().len(), 65);
        assert_eq!(key.public_point()[0], 0x04);

        // A second load with the same name should open the same key rather
        // than minting a new one.
        let again = load_or_create_root_key(Arc::clone(&store), "blueflame-test-signing-key")
            .expect("load or create again");
        assert_eq!(key.public_point(), again.public_point());
    }

    #[test]
    fn issuer_from_fake_signing_key_signs_a_leaf_cert() {
        use rcgen::{CertificateParams, DistinguishedName, DnType, Issuer};

        let store: Arc<dyn KeyStore> = Arc::new(testing::FakeKeyStore::new());
        let root_key =
            load_or_create_root_key(store, "blueflame-test-issuer-root").expect("root key");

        let mut root_params = CertificateParams::default();
        root_params.distinguished_name = DistinguishedName::new();
        root_params
            .distinguished_name
            .push(DnType::CommonName, "BlueFlame Test Root");
        root_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let root_cert = root_params
            .self_signed(&root_key)
            .expect("self-sign root with fake TPM-backed key");

        let issuer =
            Issuer::from_ca_cert_pem(&root_cert.pem(), root_key).expect("issuer from cert pem");

        let leaf_key = rcgen::KeyPair::generate().expect("leaf key");
        let mut leaf_params = CertificateParams::default();
        leaf_params.distinguished_name = DistinguishedName::new();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "example.invalid");
        leaf_params
            .signed_by(&leaf_key, &issuer)
            .expect("leaf cert signed through the fake TPM-backed issuer");
    }
}

#[cfg(all(test, target_os = "windows"))]
mod real_cng_tests {
    use super::*;

    struct DeleteOnDrop<'a> {
        store: &'a CngKeyStore,
        record: Option<KeyRecord>,
    }

    impl Drop for DeleteOnDrop<'_> {
        fn drop(&mut self) {
            if let Some(record) = self.record.take() {
                let _ = self.store.delete(record);
            }
        }
    }

    /// Exercises the real NCrypt FFI end to end: create a persisted key
    /// (TPM if present, software provider otherwise), sign a hash with it,
    /// verify that signature independently with `ring`, reopen the key by
    /// name, then delete it. Always uses a `blueflame-test-` name and always
    /// deletes what it creates, even on failure, so it never leaves
    /// anything behind in the real key store.
    #[test]
    fn creates_signs_and_deletes_a_real_persisted_key() {
        let store = CngKeyStore::new();
        let name = format!("blueflame-test-{}-{}", std::process::id(), line!());

        // Defensive cleanup in case a previous run of this test crashed
        // before it could delete its own key.
        if let Ok(Some(leftover)) = store.open(&name) {
            let _ = store.delete(leftover);
        }

        let record = store.create(&name).expect("create a persisted test key");
        let mut guard = DeleteOnDrop {
            store: &store,
            record: Some(record),
        };
        let record = guard.record.expect("record set");

        eprintln!(
            "blueflame ca_tpm test: key backend = {}",
            record.backend.as_str()
        );

        let msg = b"blueflame ca key in tpm test message";
        let hash = ring::digest::digest(&ring::digest::SHA256, msg);
        let hash32: [u8; 32] = hash.as_ref().try_into().unwrap();
        let sig = store
            .sign(&record, &hash32)
            .expect("sign with the persisted key");
        let der_sig = der_encode_ecdsa_sig(&sig);

        let public_key = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            record.point,
        );
        public_key
            .verify(msg, &der_sig)
            .expect("signature should verify against the exported public key");

        let reopened = store
            .open(&name)
            .expect("open should succeed")
            .expect("key should still exist");
        assert_eq!(reopened.point, record.point);

        let record = guard.record.take().expect("record set");
        store.delete(record).expect("delete the persisted test key");
        assert!(store.open(&name).expect("open after delete").is_none());
    }
}
