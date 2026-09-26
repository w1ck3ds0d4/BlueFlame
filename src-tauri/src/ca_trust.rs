//! Platform-specific trust-store integration for the BlueFlame root CA.
//!
//! The proxy signs per-host leaf certs with our root CA, so the OS trust
//! store needs to recognize that CA for HTTPS sites to load without
//! warnings. We never auto-install silently - every path here either runs
//! a user-scoped command that shows an OS confirmation dialog, or opens
//! the file for the user to install by hand.

use std::path::{Path, PathBuf};

/// Whether the BlueFlame root CA at `cert_path` is currently trusted by the
/// user's OS. Matched by SHA-1 thumbprint, never by common name: during a
/// TPM-key migration the legacy and new CNG-backed roots share the same
/// common name, so a common-name match cannot tell which of the two is
/// actually the one trusted.
pub fn is_trusted(cert_path: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        let Ok(pem) = std::fs::read_to_string(cert_path) else {
            return false;
        };
        let Ok(thumbprint) = cert_thumbprint_sha1(&pem) else {
            return false;
        };
        windows::is_trusted_by_thumbprint(&thumbprint)
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Reliable cross-platform trust detection needs reading and parsing
        // the OS-specific trust database - deferred until we need it.
        let _ = cert_path;
        false
    }
}

/// SHA-1 thumbprint of a PEM-encoded cert, matching what Windows' cert
/// store and `certutil` use to identify a specific cert. Shared by the
/// trust-status check above and by `ca.rs`'s legacy-root migration, so
/// there is exactly one place that computes a "thumbprint" and every
/// caller means the same thing by it.
#[cfg(target_os = "windows")]
pub fn cert_thumbprint_sha1(cert_pem: &str) -> anyhow::Result<String> {
    use anyhow::Context;
    let parsed = pem::parse(cert_pem).context("parsing PEM")?;
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, parsed.contents());
    Ok(digest.as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

/// Try to install the CA into the user's trust store. Returns ok on success.
/// On Windows this runs `certutil -user -addstore Root ...` which pops a
/// confirmation dialog the first time. No admin required.
///
/// On other platforms this returns an error with the manual install command.
pub fn install(cert_path: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows::install(cert_path)
    }

    #[cfg(target_os = "macos")]
    {
        let _ = cert_path;
        anyhow::bail!(
            "macOS auto-install is not supported yet. Run: sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain {}",
            cert_path.display()
        )
    }

    #[cfg(target_os = "linux")]
    {
        let _ = cert_path;
        anyhow::bail!(
            "Linux auto-install is not supported yet. Run: sudo cp {} /usr/local/share/ca-certificates/ && sudo update-ca-certificates",
            cert_path.display()
        )
    }
}

/// Remove a specific cert from the user's Root store by SHA-1 thumbprint.
/// Deliberately thumbprint-based rather than common-name-based: migrating
/// to a new CA root must never take out some other cert that happens to
/// share the BlueFlame CA's CN.
///
/// Only meaningful on Windows: it exists solely for the CNG migration path
/// in `ca.rs`, which is itself Windows-only (see `ca_tpm.rs`). Other
/// platforms have no caller for this yet, unlike `install` above, which the
/// `install_ca` Tauri command reaches on every platform.
#[cfg(target_os = "windows")]
pub fn remove_by_thumbprint(thumbprint: &str) -> anyhow::Result<()> {
    windows::remove_by_thumbprint(thumbprint)
}

/// Abstraction over the trust-store install/remove calls, so CA migration
/// logic can be unit tested without ever running `certutil` against
/// whoever's real trust store the test happens to run under.
#[cfg(target_os = "windows")]
pub trait TrustOps: Send + Sync {
    fn install(&self, cert_path: &Path) -> anyhow::Result<()>;
    fn remove_by_thumbprint(&self, thumbprint: &str) -> anyhow::Result<()>;
    /// Whether a cert with this thumbprint is currently trusted. Used to
    /// confirm a newly installed root actually took, and to check whether
    /// an old root still needs removing (rather than assuming it does),
    /// so a migration retry never repeats work an earlier, interrupted
    /// attempt already finished.
    fn is_trusted(&self, thumbprint: &str) -> bool;
}

/// The real, `certutil`-backed implementation used in production.
#[cfg(target_os = "windows")]
pub struct RealTrust;

#[cfg(target_os = "windows")]
impl TrustOps for RealTrust {
    fn install(&self, cert_path: &Path) -> anyhow::Result<()> {
        install(cert_path)
    }

    fn remove_by_thumbprint(&self, thumbprint: &str) -> anyhow::Result<()> {
        remove_by_thumbprint(thumbprint)
    }

    fn is_trusted(&self, thumbprint: &str) -> bool {
        windows::is_trusted_by_thumbprint(thumbprint)
    }
}

#[cfg(target_os = "windows")]
#[cfg(test)]
pub mod testing {
    use super::{cert_thumbprint_sha1, TrustOps};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// An in-memory stand-in for the OS trust store: a set of thumbprints
    /// it currently "trusts", plus optional one-call-at-a-time failure
    /// injection so a test can simulate a step failing (or the process
    /// crashing right after that step) and then retrying. Never touches a
    /// real trust store, so tests never see a Windows confirmation dialog
    /// and never depend on whatever certs happen to be on the test machine.
    #[derive(Default)]
    pub struct FakeTrust {
        /// Thumbprints this fake store currently trusts. Seed it in a test
        /// to simulate "this cert was already trusted before the test
        /// started" (e.g. the legacy root).
        pub trusted: Mutex<HashSet<String>>,
        pub install_calls: Mutex<Vec<PathBuf>>,
        pub removed: Mutex<Vec<String>>,
        /// When true, `install` fails without changing `trusted`. Flip it
        /// back to false between two calls in the same test to simulate a
        /// failed (or interrupted) attempt followed by a successful retry.
        pub fail_install: Mutex<bool>,
        /// When true, `is_trusted` reports `false` for every thumbprint
        /// regardless of `trusted`, simulating the post-install
        /// verification step failing (or the process being killed before
        /// it could run).
        pub fail_is_trusted: Mutex<bool>,
        /// When true, `remove_by_thumbprint` fails without changing
        /// `trusted`.
        pub fail_remove: Mutex<bool>,
    }

    impl FakeTrust {
        /// Pre-populate the fake store as already trusting `thumbprint`,
        /// e.g. the legacy root that was installed before this test began.
        pub fn seed_trusted(thumbprint: impl Into<String>) -> Self {
            let trust = Self::default();
            trust.trusted.lock().unwrap().insert(thumbprint.into());
            trust
        }

        pub fn set_fail_install(&self, fail: bool) {
            *self.fail_install.lock().unwrap() = fail;
        }

        pub fn set_fail_is_trusted(&self, fail: bool) {
            *self.fail_is_trusted.lock().unwrap() = fail;
        }

        pub fn set_fail_remove(&self, fail: bool) {
            *self.fail_remove.lock().unwrap() = fail;
        }
    }

    impl TrustOps for FakeTrust {
        fn install(&self, cert_path: &Path) -> anyhow::Result<()> {
            if *self.fail_install.lock().unwrap() {
                anyhow::bail!("simulated trust-store install failure");
            }
            let pem = std::fs::read_to_string(cert_path)?;
            let thumbprint = cert_thumbprint_sha1(&pem)?;
            self.trusted.lock().unwrap().insert(thumbprint);
            self.install_calls
                .lock()
                .unwrap()
                .push(cert_path.to_path_buf());
            Ok(())
        }

        fn remove_by_thumbprint(&self, thumbprint: &str) -> anyhow::Result<()> {
            if *self.fail_remove.lock().unwrap() {
                anyhow::bail!("simulated trust-store remove failure");
            }
            // Real `certutil -delstore` fails when the thumbprint is not
            // present, so match that here rather than silently succeeding.
            if !self.trusted.lock().unwrap().remove(thumbprint) {
                anyhow::bail!("no cert with thumbprint {thumbprint} in the fake trust store");
            }
            self.removed.lock().unwrap().push(thumbprint.to_string());
            Ok(())
        }

        fn is_trusted(&self, thumbprint: &str) -> bool {
            if *self.fail_is_trusted.lock().unwrap() {
                return false;
            }
            self.trusted.lock().unwrap().contains(thumbprint)
        }
    }
}

/// Open the folder containing the cert with the file selected.
pub fn reveal(cert_path: &Path) -> anyhow::Result<PathBuf> {
    let dir = cert_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cert path has no parent"))?
        .to_path_buf();

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(cert_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to launch explorer: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(cert_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to launch Finder: {e}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to launch file manager: {e}"))?;
    }

    Ok(dir)
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;

    /// Whether a cert with this SHA-1 thumbprint is in the user's Root
    /// store. `certutil -verifystore` accepts a thumbprint as the cert
    /// identifier, not just a common name, and exits 0 when found.
    pub fn is_trusted_by_thumbprint(thumbprint: &str) -> bool {
        match std::process::Command::new("certutil")
            .args(["-verifystore", "-user", "Root", thumbprint])
            .output()
        {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }

    pub fn install(cert_path: &Path) -> anyhow::Result<()> {
        let out = std::process::Command::new("certutil")
            .args(["-user", "-addstore", "Root", &cert_path.to_string_lossy()])
            .output()
            .map_err(|e| anyhow::anyhow!("failed to run certutil: {e}"))?;

        if out.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        anyhow::bail!(
            "certutil exited with {}: {}",
            out.status,
            if !stderr.trim().is_empty() {
                stderr
            } else {
                stdout
            }
        )
    }

    /// `certutil -delstore` matches on thumbprint (or CN, which we
    /// deliberately never pass here) against the user-scope Root store.
    pub fn remove_by_thumbprint(thumbprint: &str) -> anyhow::Result<()> {
        let out = std::process::Command::new("certutil")
            .args(["-user", "-delstore", "Root", thumbprint])
            .output()
            .map_err(|e| anyhow::anyhow!("failed to run certutil: {e}"))?;

        if out.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        anyhow::bail!(
            "certutil exited with {}: {}",
            out.status,
            if !stderr.trim().is_empty() {
                stderr
            } else {
                stdout
            }
        )
    }
}

#[cfg(target_os = "windows")]
#[cfg(test)]
mod tests {
    use super::testing::FakeTrust;
    use super::{cert_thumbprint_sha1, TrustOps};

    /// A self-signed cert with the given common name, so tests can build
    /// two certs that share a name but not a key (and so not a thumbprint).
    fn self_signed_cert_pem(common_name: &str) -> String {
        use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

        let key = KeyPair::generate().expect("generate key");
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.self_signed(&key).expect("self sign").pem()
    }

    #[test]
    fn same_common_name_certs_have_different_thumbprints() {
        let cert_a = self_signed_cert_pem("BlueFlame Root CA");
        let cert_b = self_signed_cert_pem("BlueFlame Root CA");

        let thumb_a = cert_thumbprint_sha1(&cert_a).expect("thumbprint a");
        let thumb_b = cert_thumbprint_sha1(&cert_b).expect("thumbprint b");

        assert_ne!(
            thumb_a, thumb_b,
            "two different certs must not collide on thumbprint just because they share a CN"
        );
    }

    #[test]
    fn is_trusted_matches_by_thumbprint_not_common_name() {
        // Two certs sharing the BlueFlame CA's common name, as the legacy
        // and CNG-backed roots do during migration - a CN-based check could
        // not tell them apart, a thumbprint-based one must.
        let legacy_cert = self_signed_cert_pem("BlueFlame Root CA");
        let new_cert = self_signed_cert_pem("BlueFlame Root CA");

        let legacy_thumb = cert_thumbprint_sha1(&legacy_cert).expect("legacy thumbprint");
        let new_thumb = cert_thumbprint_sha1(&new_cert).expect("new thumbprint");
        assert_ne!(legacy_thumb, new_thumb);

        // Only the legacy cert is trusted in this fake store.
        let trust = FakeTrust::seed_trusted(legacy_thumb.clone());

        assert!(
            trust.is_trusted(&legacy_thumb),
            "the cert actually in the store should be reported trusted"
        );
        assert!(
            !trust.is_trusted(&new_thumb),
            "a different cert with the same common name must not be reported trusted"
        );
    }
}
