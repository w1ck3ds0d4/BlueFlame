//! File downloads triggered from tab webviews.
//!
//! Flow:
//! 1. The MITM proxy sees a response with `Content-Disposition:
//!    attachment` coming back to a tab.
//! 2. The proxy streams the body to a temp file in the OS download
//!    directory, chunk by chunk, so memory use stays flat no matter
//!    how large the file is. Progress events go out over
//!    `PROGRESS_EVENT` as bytes land.
//! 3. Once the body ends, the temp file is renamed into place under
//!    a sanitized + uniquified filename (preferring the
//!    Content-Disposition name, falling back to the URL path, then
//!    to `"download"`). If the download is cancelled or the body
//!    read fails partway, the temp file is deleted instead.
//! 4. Instead of forwarding the binary to the tab (which would
//!    trigger the webview's own save dialog and we'd lose the
//!    event), the proxy substitutes a small HTML confirmation page
//!    that navigates the tab to a "Saved to..." stub with links
//!    the user can click through us.
//! 5. The entry goes into `DownloadsLog` so the Settings/Downloads
//!    UI can list recent saves.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use http_body::Body as HttpBody;
use http_body_util::BodyExt;
use serde::Serialize;
use tauri::{Emitter, Manager};
use tokio::io::AsyncWriteExt;

/// One saved download. Serialized to the frontend for the
/// Downloads view; also returned by `save_streaming()` so the proxy
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

    /// Reserve the next id for a download. Public within the crate
    /// so the proxy can assign an id up front (needed to name the
    /// temp file and register a cancel flag) before the entry
    /// itself exists.
    pub(crate) fn issue_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }
}

pub type SharedDownloadsLog = Arc<DownloadsLog>;

/// Registry of cancellation flags for downloads currently streaming
/// to disk, keyed by the id the entry will get once it lands in the
/// log. The proxy checks the flag between chunks; the frontend flips
/// it via the `downloads_cancel` command.
#[derive(Default)]
pub struct ActiveDownloads {
    inner: RwLock<HashMap<u64, Arc<AtomicBool>>>,
}

impl ActiveDownloads {
    fn register(&self, id: u64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.inner
            .write()
            .expect("active downloads poisoned")
            .insert(id, flag.clone());
        flag
    }

    /// Flip the cancel flag for `id`. Returns `false` if no download
    /// with that id is currently streaming (already finished, failed,
    /// or never existed).
    pub fn cancel(&self, id: u64) -> bool {
        match self
            .inner
            .read()
            .expect("active downloads poisoned")
            .get(&id)
        {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    fn unregister(&self, id: u64) {
        self.inner
            .write()
            .expect("active downloads poisoned")
            .remove(&id);
    }
}

pub type SharedActiveDownloads = Arc<ActiveDownloads>;

/// Event name the frontend subscribes to for live download progress.
pub const PROGRESS_EVENT: &str = "blueflame:download-progress";

/// Progress update emitted as a download streams to disk. `total` is
/// `None` when the server did not send a `Content-Length` (chunked
/// transfer) - the UI falls back to a spinner instead of a
/// percentage in that case. `done` is `true` on the final event for
/// a given `id`, whether it finished, was cancelled, or failed.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub id: u64,
    pub url: String,
    pub bytes_written: u64,
    pub total: Option<u64>,
    pub done: bool,
}

fn emit_progress(app: &tauri::AppHandle, progress: &DownloadProgress) {
    let _ = app.emit(PROGRESS_EVENT, progress);
}

/// Why a streaming download did not end with a saved file.
#[derive(Debug, PartialEq, Eq)]
pub enum DownloadFailure {
    /// Cancelled via `downloads_cancel` (or the flag was already set
    /// before the first chunk arrived).
    Cancelled,
    /// A body-read error, an I/O error writing or renaming the temp
    /// file, or a failure creating the destination directory.
    Failed(String),
}

impl std::fmt::Display for DownloadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadFailure::Cancelled => write!(f, "download cancelled"),
            DownloadFailure::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

/// Stream `body` to a temp file under `dest_dir`, calling
/// `on_progress` after every chunk, then rename it into place under
/// a sanitized + uniquified filename. There is no size cap: the body
/// is written incrementally so memory use stays flat regardless of
/// file size.
///
/// On cancellation (`cancel` flips to `true`) or any read/write
/// error, the partial temp file is deleted and no entry is added to
/// the log.
#[allow(clippy::too_many_arguments)]
pub async fn stream_to_disk<B>(
    log: &DownloadsLog,
    dest_dir: &Path,
    id: u64,
    url: &str,
    disposition_filename: Option<&str>,
    declared_len: Option<u64>,
    mut body: B,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<DownloadEntry, DownloadFailure>
where
    B: HttpBody<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| DownloadFailure::Failed(format!("create download dir: {e}")))?;

    let raw_name = disposition_filename
        .map(|s| s.to_string())
        .or_else(|| filename_from_url(url))
        .unwrap_or_else(|| "download".to_string());
    let safe = sanitize_filename(&raw_name);

    // Write under a hidden temp name first so a half-written file
    // never shows up under its real name, and never collides with
    // `unique_path`'s check against already-finished downloads.
    let temp_path = dest_dir.join(format!(".blueflame-download-{id}.part"));

    let mut file = tokio::fs::File::create(&temp_path)
        .await
        .map_err(|e| DownloadFailure::Failed(format!("create temp file: {e}")))?;

    let mut bytes_written: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(DownloadFailure::Cancelled);
        }

        let frame = match body.frame().await {
            None => break,
            Some(Ok(frame)) => frame,
            Some(Err(e)) => {
                drop(file);
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(DownloadFailure::Failed(format!("read body: {e}")));
            }
        };

        let data = match frame.into_data() {
            Ok(data) => data,
            // Trailer frame carrying no body bytes - nothing to write.
            Err(_) => continue,
        };

        if let Err(e) = file.write_all(&data).await {
            drop(file);
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(DownloadFailure::Failed(format!("write file: {e}")));
        }
        bytes_written += data.len() as u64;
        on_progress(bytes_written, declared_len);
    }

    if let Err(e) = file.flush().await {
        drop(file);
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(DownloadFailure::Failed(format!("flush file: {e}")));
    }
    drop(file);

    let final_path = unique_path(dest_dir, &safe);
    if let Err(e) = tokio::fs::rename(&temp_path, &final_path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(DownloadFailure::Failed(format!("finalize file: {e}")));
    }

    let filename = final_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| safe.clone());

    let entry = DownloadEntry {
        id,
        url: url.to_string(),
        filename,
        path: final_path.to_string_lossy().into_owned(),
        size: bytes_written,
        ts: now_secs(),
    };
    log.add(entry.clone());
    Ok(entry)
}

/// Resolve the OS download directory and stream a response body to
/// disk under it, wiring progress events to the frontend and
/// registering a cancel flag the `downloads_cancel` command can flip
/// while the download is in flight.
pub async fn save_streaming<B>(
    log: &DownloadsLog,
    active: &ActiveDownloads,
    app: &tauri::AppHandle,
    url: &str,
    disposition_filename: Option<&str>,
    declared_len: Option<u64>,
    body: B,
) -> Result<DownloadEntry, DownloadFailure>
where
    B: HttpBody<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| DownloadFailure::Failed(format!("no download dir available: {e}")))?;

    let id = log.issue_id();
    let cancel = active.register(id);
    let url_owned = url.to_string();

    let result = stream_to_disk(
        log,
        &dir,
        id,
        url,
        disposition_filename,
        declared_len,
        body,
        &cancel,
        |bytes_written, total| {
            emit_progress(
                app,
                &DownloadProgress {
                    id,
                    url: url_owned.clone(),
                    bytes_written,
                    total,
                    done: false,
                },
            );
        },
    )
    .await;

    active.unregister(id);
    emit_progress(
        app,
        &DownloadProgress {
            id,
            url: url.to_string(),
            bytes_written: result.as_ref().map(|e| e.size).unwrap_or(0),
            total: declared_len,
            done: true,
        },
    );
    result
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

// -- Commands surfaced to the frontend --------------------------------

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

/// Cancel a download that is currently streaming to disk. Returns
/// `false` if no such download is in flight (already finished,
/// already failed, or an unknown id).
#[tauri::command]
pub fn downloads_cancel(active: tauri::State<'_, SharedActiveDownloads>, id: u64) -> bool {
    active.cancel(id)
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
    use http_body_util::StreamBody;

    /// Build a streaming body out of fixed chunks, useful for
    /// exercising `stream_to_disk` without a real network response.
    /// The last chunk, if `Err`, simulates a connection that dies
    /// partway through (an interrupted download).
    fn chunked_body(
        chunks: Vec<Result<&'static [u8], &'static str>>,
    ) -> impl HttpBody<Data = Bytes, Error = String> + Unpin {
        let frames = chunks.into_iter().map(|c| {
            c.map(|d| http_body::Frame::data(Bytes::from_static(d)))
                .map_err(|e| e.to_string())
        });
        StreamBody::new(futures_util::stream::iter(frames))
    }

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

    #[tokio::test]
    async fn stream_to_disk_writes_all_chunks_and_records() {
        let log = DownloadsLog::default();
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let body = chunked_body(vec![Ok(b"hello "), Ok(b"streaming "), Ok(b"world")]);
        let mut progress_calls = Vec::new();

        let entry = stream_to_disk(
            &log,
            dir.path(),
            1,
            "https://example/hello.bin",
            None,
            Some(21),
            body,
            &cancel,
            |written, total| progress_calls.push((written, total)),
        )
        .await
        .expect("stream should succeed");

        assert_eq!(entry.size, 21);
        assert_eq!(
            std::fs::read(&entry.path).unwrap(),
            b"hello streaming world"
        );
        // No leftover temp file.
        assert!(!dir.path().join(".blueflame-download-1.part").exists());
        // Progress reported after every chunk, cumulative and
        // matching the declared total.
        assert_eq!(
            progress_calls,
            vec![(6, Some(21)), (16, Some(21)), (21, Some(21))]
        );
        assert_eq!(log.recent(10).len(), 1);
    }

    #[tokio::test]
    async fn stream_to_disk_has_no_size_cap() {
        // A body larger than the old 500 MB MAX_SIZE constant must
        // stream through fine now that nothing buffers it whole.
        let log = DownloadsLog::default();
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let chunk: &'static [u8] = &[7u8; 8 * 1024 * 1024]; // 8 MiB chunk
        let chunk_count = 65; // ~520 MiB total, over the old cap
        let chunks: Vec<Result<&'static [u8], &'static str>> =
            std::iter::repeat_n(Ok(chunk), chunk_count).collect();
        let body = chunked_body(chunks);

        let entry = stream_to_disk(
            &log,
            dir.path(),
            2,
            "https://example/big.bin",
            None,
            None,
            body,
            &cancel,
            |_, _| {},
        )
        .await
        .expect("large stream should succeed with no cap");

        assert_eq!(entry.size, (chunk.len() * chunk_count) as u64);
    }

    #[tokio::test]
    async fn stream_to_disk_cleans_up_on_cancel() {
        let log = DownloadsLog::default();
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(true); // already cancelled before the first chunk
        let body = chunked_body(vec![Ok(b"never written")]);

        let err = stream_to_disk(
            &log,
            dir.path(),
            3,
            "https://example/cancelled.bin",
            None,
            None,
            body,
            &cancel,
            |_, _| {},
        )
        .await
        .expect_err("cancelled stream should fail");

        assert_eq!(err, DownloadFailure::Cancelled);
        assert!(!dir.path().join(".blueflame-download-3.part").exists());
        // Nothing else in the directory either - no partial file left
        // under any name.
        let remaining: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(remaining.is_empty());
        assert_eq!(log.recent(10).len(), 0);
    }

    #[tokio::test]
    async fn stream_to_disk_cleans_up_mid_stream_cancel() {
        let log = DownloadsLog::default();
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        // Cancel after the first chunk lands, before the second is read.
        let chunks = vec![Ok(&b"first"[..]), Ok(&b"second"[..])];
        let body = chunked_body(chunks);
        let mut seen_first = false;

        // Drive the stream manually so we can flip the flag between
        // chunks, mirroring a user clicking cancel mid-download.
        let cancel_ref = &cancel;
        let result = stream_to_disk(
            &log,
            dir.path(),
            4,
            "https://example/mid-cancel.bin",
            None,
            None,
            body,
            cancel_ref,
            |_, _| {
                if !seen_first {
                    seen_first = true;
                    cancel_ref.store(true, Ordering::Relaxed);
                }
            },
        )
        .await;

        assert_eq!(result.unwrap_err(), DownloadFailure::Cancelled);
        assert!(!dir.path().join(".blueflame-download-4.part").exists());
        assert_eq!(log.recent(10).len(), 0);
    }

    #[tokio::test]
    async fn stream_to_disk_cleans_up_on_interrupted_body() {
        // Simulates a dropped connection: the body yields some bytes
        // then an error frame instead of ending cleanly.
        let log = DownloadsLog::default();
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let body = chunked_body(vec![Ok(b"partial data"), Err("connection reset")]);

        let err = stream_to_disk(
            &log,
            dir.path(),
            5,
            "https://example/interrupted.bin",
            None,
            Some(1000),
            body,
            &cancel,
            |_, _| {},
        )
        .await
        .expect_err("interrupted stream should fail");

        match err {
            DownloadFailure::Failed(msg) => assert!(msg.contains("connection reset")),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(!dir.path().join(".blueflame-download-5.part").exists());
        let remaining: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(remaining.is_empty());
        assert_eq!(log.recent(10).len(), 0);
    }

    #[test]
    fn active_downloads_cancel_flips_flag_and_reports_unknown() {
        let active = ActiveDownloads::default();
        let flag = active.register(9);
        assert!(!flag.load(Ordering::Relaxed));
        assert!(active.cancel(9));
        assert!(flag.load(Ordering::Relaxed));
        // Cancelling an id that was never registered (or already
        // finished and unregistered) reports false rather than
        // panicking.
        assert!(!active.cancel(404));
        active.unregister(9);
        assert!(!active.cancel(9));
    }
}
