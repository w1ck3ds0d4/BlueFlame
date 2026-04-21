//! Favicon cache for the tab strip.
//!
//! Fetches `/favicon.ico` for each navigated host, stashes the bytes on disk
//! under `<app_data>/favicons/<safe-host>.dat`, and hands base64 data URLs
//! to the UI on demand. Fetches go direct (not through the MITM proxy) to
//! avoid recursive loops through our own filter.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;

/// One cache entry on disk: first line is the mime type, rest is raw bytes.
/// Compact enough that we don't need an index file.
const MIME_SEPARATOR: &[u8] = b"\n---\n";

/// Max favicon size we accept. 256 KiB is plenty; anything larger is almost
/// certainly not an icon.
const MAX_BYTES: usize = 256 * 1024;

pub fn dir(app_data: &Path) -> PathBuf {
    app_data.join("favicons")
}

fn safe_host(host: &str) -> String {
    host.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn path_for(app_data: &Path, host: &str) -> PathBuf {
    dir(app_data).join(format!("{}.dat", safe_host(host)))
}

/// Read a cached favicon if we have one.
pub fn read(app_data: &Path, host: &str) -> Option<(String, Vec<u8>)> {
    let p = path_for(app_data, host);
    let bytes = std::fs::read(&p).ok()?;
    let split = find_subslice(&bytes, MIME_SEPARATOR)?;
    let mime = std::str::from_utf8(&bytes[..split]).ok()?.to_string();
    let body = bytes[split + MIME_SEPARATOR.len()..].to_vec();
    if body.is_empty() {
        return None;
    }
    Some((mime, body))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

pub fn write(app_data: &Path, host: &str, mime: &str, body: &[u8]) -> anyhow::Result<()> {
    let d = dir(app_data);
    std::fs::create_dir_all(&d).context("creating favicons dir")?;
    let p = path_for(app_data, host);
    let mut payload = Vec::with_capacity(mime.len() + MIME_SEPARATOR.len() + body.len());
    payload.extend_from_slice(mime.as_bytes());
    payload.extend_from_slice(MIME_SEPARATOR);
    payload.extend_from_slice(body);
    let tmp = p.with_extension("dat.tmp");
    std::fs::write(&tmp, &payload)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// Indicate that a previous fetch for this host failed so the caller doesn't
/// re-fetch on every single navigation. Stored as a zero-length body.
pub fn mark_miss(app_data: &Path, host: &str) -> anyhow::Result<()> {
    write(app_data, host, "x-miss", &[])
}

/// Whether we already have any disposition (hit or miss) cached for a host.
pub fn is_cached(app_data: &Path, host: &str) -> bool {
    path_for(app_data, host).exists()
}

/// Fetch `https://<host>/favicon.ico` and cache the bytes. Errors are
/// logged and converted to a miss-marker so we don't retry every request.
pub async fn fetch_and_cache(app_data: &Path, host: &str) -> anyhow::Result<()> {
    // A stock reqwest client builds its own connector pool and does not
    // pick up the WebView2 proxy env var, so favicon requests go direct.
    let client = reqwest::Client::builder()
        .user_agent("BlueFlame/0.1 (favicon-fetcher)")
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .context("building favicon http client")?;

    let url = format!("https://{host}/favicon.ico");
    let resp = client.get(&url).send().await;
    match resp {
        Ok(r) if r.status().is_success() => {
            let mime = r
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(';').next().unwrap_or("").trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "image/x-icon".to_string());
            let bytes = r.bytes().await.context("reading favicon body")?;
            if bytes.is_empty() || bytes.len() > MAX_BYTES {
                mark_miss(app_data, host).ok();
                return Ok(());
            }
            write(app_data, host, &mime, &bytes)?;
        }
        _ => {
            mark_miss(app_data, host).ok();
        }
    }
    Ok(())
}

/// Base64-encoded data URL suitable for dropping straight into an `<img src>`.
pub fn data_url(mime: &str, body: &[u8]) -> String {
    format!("data:{mime};base64,{}", crate::util::base64_encode(body))
}
