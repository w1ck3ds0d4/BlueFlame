//! Small shared utilities.

/// Minimal base64 encoder. Standard RFC 4648 alphabet with `=` padding.
pub fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut chunks = input.chunks_exact(3);
    for chunk in chunks.by_ref() {
        let b = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((b >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(b & 0x3f) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        1 => {
            let b = u32::from(rem[0]) << 16;
            out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b = (u32::from(rem[0]) << 16) | (u32::from(rem[1]) << 8);
            out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}
