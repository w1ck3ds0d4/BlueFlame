//! Per-session control-channel token.
//!
//! BlueFlame generates one random token per app launch and writes it, hex
//! encoded, to a file under the app data directory. The MCP bridge reads
//! that file and sends the token as the first message on the named pipe.
//! The pipe server refuses every request that does not carry a byte-for-byte
//! match, compared in constant time so a partial guess can't be timed.

use std::path::{Path, PathBuf};

/// 256 bits, hex encoded to 64 ASCII characters.
const TOKEN_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    /// Generate a fresh random token from the OS CSPRNG.
    pub fn generate() -> Self {
        let mut buf = [0u8; TOKEN_BYTES];
        getrandom::getrandom(&mut buf).expect("OS RNG unavailable");
        Self(to_hex(&buf))
    }

    #[cfg(test)]
    pub fn from_str_for_test(s: &str) -> Self {
        Self(s.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison against a caller-supplied string. Always
    /// walks the full length of `self` regardless of where a mismatch
    /// occurs, so a bad guess can't be timed byte-by-byte.
    pub fn matches(&self, candidate: &str) -> bool {
        let a = self.0.as_bytes();
        let b = candidate.as_bytes();
        if a.len() != b.len() {
            // Still touch every byte of `a` so the early-return itself
            // doesn't leak length information through timing beyond what
            // the length check above already reveals.
            let mut acc: u8 = 1;
            for byte in a {
                acc |= *byte;
            }
            let _ = acc;
            return false;
        }
        let mut diff: u8 = 0;
        for i in 0..a.len() {
            diff |= a[i] ^ b[i];
        }
        diff == 0
    }
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn control_dir(app_data: &Path) -> PathBuf {
    app_data.join("control")
}

pub fn token_file_path(app_data: &Path) -> PathBuf {
    control_dir(app_data).join("token")
}

/// Write the token to `<app_data>/control/token`. The caller (the Windows
/// pipe-server setup path) is responsible for locking the file down to the
/// current user afterward; this function only handles the plain write so
/// it stays testable on every OS.
pub fn write_token_file(app_data: &Path, token: &SessionToken) -> std::io::Result<PathBuf> {
    let dir = control_dir(app_data);
    std::fs::create_dir_all(&dir)?;
    let path = token_file_path(app_data);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, token.as_str())?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_64_hex_chars() {
        let t = SessionToken::generate();
        assert_eq!(t.as_str().len(), TOKEN_BYTES * 2);
        assert!(t.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_generated_tokens_differ() {
        assert_ne!(
            SessionToken::generate().as_str(),
            SessionToken::generate().as_str()
        );
    }

    #[test]
    fn matches_exact_string() {
        let t = SessionToken::from_str_for_test("abc123");
        assert!(t.matches("abc123"));
    }

    #[test]
    fn rejects_wrong_value() {
        let t = SessionToken::from_str_for_test("abc123");
        assert!(!t.matches("abc124"));
        assert!(!t.matches("abc12"));
        assert!(!t.matches(""));
        assert!(!t.matches("abc1234"));
    }

    #[test]
    fn write_token_file_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let token = SessionToken::from_str_for_test("deadbeef");
        let path = write_token_file(dir.path(), &token).unwrap();
        let read_back = std::fs::read_to_string(path).unwrap();
        assert_eq!(read_back, "deadbeef");
    }
}
