//! Windows ACL helpers: restrict the control pipe and the token file to the
//! current user only, nobody else, not even an administrator.
//!
//! Both use the same technique: an SDDL string naming this process's own
//! user SID (`D:P(A;;GA;;;<sid>)` - a protected DACL, one ACE, generic-all,
//! that SID only), converted to a real security descriptor with
//! `ConvertStringSecurityDescriptorToSecurityDescriptorW`.

use std::path::Path;

use windows::core::{Result as WinResult, PWSTR};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The current process's user SID, as an SDDL string like
/// `S-1-5-21-...-1001`. Every ACL this module builds names this SID and
/// nothing else.
fn current_user_sid_string() -> WinResult<String> {
    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let _guard = HandleGuard(token);

        // First call to learn the buffer size, second to fill it in - the
        // documented pattern for the TokenUser info class.
        let mut needed: u32 = 0;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        let mut buf = vec![0u8; needed as usize];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )?;
        let token_user = &*(buf.as_ptr().cast::<TOKEN_USER>());

        let mut sid_str = PWSTR::null();
        ConvertSidToStringSidW(token_user.User.Sid, &mut sid_str)?;
        let _sid_guard = LocalFreeGuard(sid_str.0 as *mut core::ffi::c_void);
        Ok(sid_str.to_string().unwrap_or_default())
    }
}

/// SDDL restricting an object to the current user only: a protected DACL
/// (so it doesn't inherit anything looser) with one ACE granting generic-all
/// to that user's SID.
pub fn current_user_only_sddl() -> WinResult<String> {
    let sid = current_user_sid_string()?;
    Ok(format!("D:P(A;;GA;;;{sid})"))
}

/// Wraps a self-relative security descriptor built from `sddl`, sized for
/// use as `lpSecurityDescriptor` in a `SECURITY_ATTRIBUTES` (e.g. for
/// `ServerOptions::create_with_security_attributes_raw`). Frees the
/// descriptor's memory on drop.
pub struct SecurityDescriptorGuard {
    psd: PSECURITY_DESCRIPTOR,
    pub attributes: SECURITY_ATTRIBUTES,
}

impl SecurityDescriptorGuard {
    pub fn from_sddl(sddl: &str) -> WinResult<Self> {
        let sddl_wide = windows::core::HSTRING::from(sddl);
        let mut psd = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl_wide,
                SDDL_REVISION_1,
                &mut psd,
                None,
            )?;
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: psd.0,
            bInheritHandle: false.into(),
        };
        Ok(Self { psd, attributes })
    }

    /// Raw pointer to the `SECURITY_ATTRIBUTES`, valid for as long as this
    /// guard lives. Used with tokio's `create_with_security_attributes_raw`.
    pub fn as_raw(&self) -> *const core::ffi::c_void {
        (&self.attributes as *const SECURITY_ATTRIBUTES).cast()
    }
}

impl Drop for SecurityDescriptorGuard {
    fn drop(&mut self) {
        if !self.psd.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.psd.0)));
            }
        }
    }
}

// SAFETY: the descriptor is heap memory with no thread affinity; only one
// owner ever touches it (this guard), matching `Send`'s requirements.
unsafe impl Send for SecurityDescriptorGuard {}

/// Restrict an existing file on disk (the token file) to the current user
/// only, by pulling the DACL back out of a `current_user_only_sddl()`
/// descriptor and applying it directly to the file's ACL entry.
pub fn lock_down_file(path: &Path) -> WinResult<()> {
    let sddl = current_user_only_sddl()?;
    let descriptor = SecurityDescriptorGuard::from_sddl(&sddl)?;

    let mut dacl_present = windows::core::BOOL(0);
    let mut dacl_ptr: *mut ACL = std::ptr::null_mut();
    let mut dacl_defaulted = windows::core::BOOL(0);
    unsafe {
        GetSecurityDescriptorDacl(
            descriptor.psd,
            &mut dacl_present,
            &mut dacl_ptr,
            &mut dacl_defaulted,
        )?;
    }

    let wide_path = windows::core::HSTRING::from(path.as_os_str());
    let result = unsafe {
        SetNamedSecurityInfoW(
            &wide_path,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl_ptr as *const ACL),
            None,
        )
    };
    result.ok()
}

struct HandleGuard(windows::Win32::Foundation::HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

struct LocalFreeGuard(*mut core::ffi::c_void);
impl Drop for LocalFreeGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_sid_string_looks_like_a_sid() {
        let sid = current_user_sid_string().expect("current user SID");
        assert!(sid.starts_with("S-1-"));
    }

    #[test]
    fn sddl_embeds_the_current_user_sid() {
        let sid = current_user_sid_string().unwrap();
        let sddl = current_user_only_sddl().unwrap();
        assert!(sddl.contains(&sid));
        assert!(sddl.starts_with("D:P("));
    }

    #[test]
    fn security_descriptor_builds_from_sddl() {
        let sddl = current_user_only_sddl().unwrap();
        let guard = SecurityDescriptorGuard::from_sddl(&sddl).unwrap();
        assert!(!guard.attributes.lpSecurityDescriptor.is_null());
    }

    #[test]
    fn lock_down_file_does_not_error_on_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blueflame-test-token");
        std::fs::write(&path, "test-token").unwrap();
        lock_down_file(&path).expect("lock down should succeed for a file we own");
    }
}
