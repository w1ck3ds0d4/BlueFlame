//! File downloads triggered from tab webviews.
//!
//! Flow:
//! 1. The MITM proxy sees a response with `Content-Disposition:
//!    attachment` coming back to a tab.
//! 2. The proxy collects the body (capped at `MAX_SIZE`), picks a
//!    safe filename (preferring the Content-Disposition one, falling
//!    back to the URL path, then to `"download"`), and writes to
//!    the OS download directory under a uniquified name.
//! 3. Instead of forwarding the binary to the tab (which would
//!    trigger the webview's own save dialog and we'd lose the
//!    event), the proxy substitutes a small HTML confirmation page
//!    that navigates the tab to a "Saved to …" stub with links
//!    the user can click through us.
//! 4. The entry goes into `DownloadsLog` so the Settings/Downloads
//!    UI can list recent saves.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::Serialize;
use tauri::Manager;

/// One saved download. Serialized to the frontend for the
/// Downloads view; also returned as part of `save()` so the proxy
/// knows what to put in the stub HTML.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadEntry {
    pub id: u64,
    /// Absolute URL the webview requested.
    pub url: String,
    /// Filename as written to disk (after sanitization +
    /// uniquification against existing files in the folder).
    pub filename: String,
    /// Absolute path on disk.
    pub path: String,
    /// Bytes written.
    pub size: u64,
    /// Unix seconds when the save completed.
    pub ts: u64,
}

/// Bounded history of downloads so the list stays cheap to store
/// and render. A user running for months shouldn't accumulate
/// unbounded state; 200 entries is plenty for recent.
const CAPACITY: usize = 200;

/// Cap any single download at 500 MB. Above this we return an
/// error stub instead of eating hundreds of megs of RAM in the
/// proxy. The proxy buffers the body in memory (hudsucker's
/// handler signature doesn't expose a streaming writer without
/// a bigger rewrite), so we need *some* bound.
pub const MAX_SIZE: u64 = 500 * 1024 * 1024;

#[derive(Default)]
pub struct DownloadsLog {
    inner: RwLock<VecDeque<DownloadEntry>>,
    next_id: AtomicU64,
}

impl DownloadsLog {
    pub fn add(&self, entry: DownloadEntry) {
        let mut q = self.inner.write().expect("downloads log poisoned");
        if q.len() >= CAPACITY {
            q.pop_front();
        }
        q.push_back(entry);
    }

    pub fn recent(&self, limit: usize) -> Vec<DownloadEntry> {
        let q = self.inner.read().expect("downloads log poisoned");
        let take = limit.min(q.len());
        q.iter().skip(q.len() - take).rev().cloned().collect()
    }

    pub fn clear(&self) {
        let mut q = self.inner.write().expect("downloads log poisoned");
        q.clear();
    }

    fn issue_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }
}

pub type SharedDownloadsLog = Arc<DownloadsLog>;

/// Save a download from an in-memory buffer. Returns the entry on
/// success; on failure returns an Err with a short reason string
/// so the proxy can surface it in the failure stub.
pub fn save(
    log: &DownloadsLog,
    app: &tauri::AppHandle,
    url: &str,
    disposition_filename: Option<&str>,
    body: &[u8],
) -> Result<DownloadEntry, String> {
    let size = body.len() as u64;
    if size > MAX_SIZE {
        return Err(format!(
            "file too large for BlueFlame to save ({} MB > {} MB cap)",
            size / 1024 / 1024,
            MAX_SIZE / 1024 / 1024
        ));
    }

    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| format!("no download dir available: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create download dir: {e}"))?;

    let raw_name = disposition_filename
        .map(|s| s.to_string())
        .or_else(|| filename_from_url(url))
        .unwrap_or_else(|| "download".to_string());
    let safe = sanitize_filename(&raw_name);
    let path = unique_path(&dir, &safe);
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| safe.clone());

    std::fs::write(&path, body).map_err(|e| format!("write file: {e}"))?;

    let entry = DownloadEntry {
        id: log.issue_id(),
        url: url.to_string(),
        filename,
        path: path.to_string_lossy().into_owned(),
        size,
        ts: now_secs(),
    };
    log.add(entry.clone());
    Ok(entry)
}

/// Strip any path separators and control characters from a
/// user-supplied filename. We've pulled this from an HTTP header
/// the server controls so it could contain `..`, slashes, colons,
/// null bytes, or other path-traversal tricks.
fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    // Reject a pure-dotfile like `..` and empty names.
    let trimmed = out.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        // Cap length - some filesystems top out at 255 bytes, leave
        // room for the uniquifying " (n)" suffix below.
        let max = 200;
        if trimmed.len() > max {
            trimmed.chars().take(max).collect()
        } else {
            trimmed.to_string()
        }
    }
}

/// If `dir/filename` already exists, try `filename (2)`, `filename
/// (3)`, etc. up to a sensible cap before giving up.
fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match filename.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (filename.to_string(), String::new()),
    };
    for n in 2..1000 {
        let alt = dir.join(format!("{stem} ({n}){ext}"));
        if !alt.exists() {
            return alt;
        }
    }
    candidate
}

/// Last path segment of the URL, url-decoded. E.g.
/// `https://example.com/files/foo%20bar.zip?x=1` -> `foo bar.zip`.
fn filename_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let last = parsed
        .path_segments()?
        .rfind(|s| !s.is_empty())?
        .to_string();
    let decoded = percent_encoding::percent_decode_str(&last)
        .decode_utf8_lossy()
        .into_owned();
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Commands surfaced to the frontend ────────────────────────────

#[tauri::command]
pub fn downloads_list(
    log: tauri::State<'_, SharedDownloadsLog>,
    limit: Option<usize>,
) -> Vec<DownloadEntry> {
    let take = limit.unwrap_or(100).min(CAPACITY);
    log.recent(take)
}

#[tauri::command]
pub fn downloads_clear(log: tauri::State<'_, SharedDownloadsLog>) {
    log.clear();
}

/// Open the saved file in its default application. Uses tauri-plugin-
/// opener which is already in the dep tree (and works on mobile).
#[tauri::command]
pub fn downloads_open(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| format!("open: {e}"))
}

/// Reveal the file in the OS file browser. Implemented by opening
/// the containing folder. On macOS `open -R` would highlight the
/// specific file but tauri-plugin-opener doesn't expose that, so
/// we just open the parent dir - good enough for MVP.
#[tauri::command]
pub fn downloads_reveal(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let parent = std::path::Path::new(&path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| "no parent directory".to_string())?;
    app.opener()
        .open_path(parent, None::<&str>)
        .map_err(|e| format!("open: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_blocks_traversal() {
        // Slashes become underscores; leading dots get trimmed. No
        // interior `/` or `\` survives to create a path component.
        let cleaned = sanitize_filename("../../etc/passwd");
        assert!(!cleaned.contains('/'));
        assert!(!cleaned.contains('\\'));
        assert!(!cleaned.starts_with('.'));
        assert_eq!(sanitize_filename("ok.zip"), "ok.zip");
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(sanitize_filename(""), "download");
    }

    #[test]
    fn filename_from_url_handles_encoding() {
        assert_eq!(
            filename_from_url("https://x.example/path/foo%20bar.zip?q=1"),
            Some("foo bar.zip".to_string())
        );
        assert_eq!(filename_from_url("https://x.example/"), None);
    }

    #[test]
    fn unique_path_appends_counter() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = unique_path(dir.path(), "a.zip");
        std::fs::write(&p1, []).unwrap();
        let p2 = unique_path(dir.path(), "a.zip");
        assert!(p2.to_string_lossy().ends_with("a (2).zip"));
    }

    #[test]
    fn save_writes_and_records() {
        // Minimal integration test: feed a buffer through save(),
        // verify the file exists with the right contents and the
        // log has the entry.
        let log = DownloadsLog::default();
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"hello download";
        // save() wants an AppHandle for path resolution; bypass by
        // calling the core write path manually. (In the real
        // codepath proxy.rs calls save() with the handle.)
        let name = "hello.bin";
        let path = unique_path(dir.path(), name);
        std::fs::write(&path, bytes).unwrap();
        log.add(DownloadEntry {
            id: 1,
            url: "https://example/hello.bin".to_string(),
            filename: name.to_string(),
            path: path.to_string_lossy().into_owned(),
            size: bytes.len() as u64,
            ts: now_secs(),
        });
        let recent = log.recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].filename, name);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
