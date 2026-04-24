//! Brand assets shared across standalone HTML surfaces (new-tab
//! page, metasearch results page). Both are rendered as self-
//! contained HTML (data URL or direct response body) so they can't
//! rely on any bundler/asset pipeline - we bake the logo into the
//! binary at compile time and render it inline as a base64 data URL.

use std::sync::OnceLock;

use crate::util::base64_encode;

/// Embedded BlueFlame logo. `include_bytes!` locks the bytes into
/// the final binary so no runtime I/O or external asset path is
/// involved.
const LOGO_BYTES: &[u8] = include_bytes!("../../src/assets/LOGO-GRADIENT.BF.png");

/// `data:image/png;base64,...` URL for the logo. Cached in a
/// `OnceLock` since the encoded string is ~66 KB and the bytes
/// never change - we don't want to re-encode on every new-tab or
/// search render.
pub fn logo_data_url() -> &'static str {
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED.get_or_init(|| format!("data:image/png;base64,{}", base64_encode(LOGO_BYTES)))
}
