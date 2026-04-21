//! Platform-specific trust-store integration for the BlueFlame root CA.
//!
//! The proxy signs per-host leaf certs with our root CA, so the OS trust
//! store needs to recognize that CA for HTTPS sites to load without
//! warnings. We never auto-install silently - every path here either runs
//! a user-scoped command that shows an OS confirmation dialog, or opens
//! the file for the user to install by hand.

use std::path::{Path, PathBuf};

/// Whether the BlueFlame root CA is currently trusted by the user's OS.
pub fn is_trusted(cert_path: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::is_trusted(cert_path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Reliable cross-platform trust detection needs reading and parsing
        // the OS-specific trust database - deferred until we need it.
        let _ = cert_path;
        false
    }
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

    const CA_COMMON_NAME: &str = "BlueFlame Root CA";

    pub fn is_trusted(_cert_path: &Path) -> bool {
        // `certutil -verifystore -user Root <CN>` exits 0 when the cert is present.
        match std::process::Command::new("certutil")
            .args(["-verifystore", "-user", "Root", CA_COMMON_NAME])
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
}
