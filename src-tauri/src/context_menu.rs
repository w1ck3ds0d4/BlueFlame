//! Right-click / long-press context menu for pages loaded in tab
//! webviews. Architecturally mirrors the existing `open_menu_popup`
//! flow (child webview popup anchored to the click position), plus a
//! small back-channel so JS running inside the tab can signal Rust
//! that a context menu should open.
//!
//! The back-channel is a sentinel host (`blueflame.ipc`) that the
//! MITM proxy intercepts: the per-tab init script calls
//! `navigator.sendBeacon('http://blueflame.ipc/...')` with the click
//! target details, the proxy matches the sentinel host + validates
//! the per-launch auth token, and pushes a `ContextMenuRequest` onto
//! an mpsc channel. A consumer task in `lib.rs` receives the request
//! and calls `open_context_menu_popup()` on the main thread.
//!
//! The popup itself is just a tiny React page (`?panel=context`)
//! that reads its payload via `get_context_payload(ctx_id)` - the
//! raw URL can be long, so we pass only the UUID through the query
//! string and keep the actual payload in an in-memory map.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use tauri::{LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl};

/// Label of the context-menu popup webview. Unique so open/close can
/// idempotently target it.
pub const CONTEXT_MENU_LABEL: &str = "context-menu";

/// Maximum popup geometry. The React popup calls `resize_context_menu`
/// with its measured height after mount, so this is just an upper
/// bound during initial creation before the measurement lands.
const POPUP_WIDTH: f64 = 240.0;
const POPUP_MAX_HEIGHT: f64 = 360.0;
const POPUP_MARGIN: f64 = 6.0;

/// Background color used on the WebviewBuilder so the popup paints
/// dark before the React bundle loads. Matches `--bg-elev` in App.css.
const POPUP_BG_COLOR: tauri::webview::Color = tauri::webview::Color(13, 13, 13, 255);

/// One right-click / long-press event captured by the tab's init
/// script. Only one of `link_url` / `image_url` / `selection_text`
/// is typically populated per event; `page_url` is always set so the
/// popup can offer page-level actions (bookmark, back, etc.) even
/// when the user clicked blank space.
#[derive(Debug, Clone, Serialize)]
pub struct ContextMenuPayload {
    pub page_url: String,
    pub link_url: Option<String>,
    pub link_text: Option<String>,
    pub image_url: Option<String>,
    pub selection_text: Option<String>,
    pub screen_x: f64,
    pub screen_y: f64,
}

/// In-memory store: UUID -> payload. The popup fetches its payload
/// via `get_context_payload(ctx_id)` on mount. Entries are cleared
/// by `take_context_payload` (one-shot) so a closed popup can't be
/// re-opened by replaying the URL.
#[derive(Default)]
pub struct ContextStore {
    entries: Mutex<HashMap<String, (ContextMenuPayload, Instant)>>,
}

impl ContextStore {
    pub fn put(&self, id: String, payload: ContextMenuPayload) {
        let mut g = self.entries.lock().expect("context store poisoned");
        // Sweep stale entries (> 60s old) so we don't leak if a popup
        // is never opened for some reason.
        let cutoff = Instant::now() - std::time::Duration::from_secs(60);
        g.retain(|_, (_, t)| *t > cutoff);
        g.insert(id, (payload, Instant::now()));
    }

    pub fn take(&self, id: &str) -> Option<ContextMenuPayload> {
        let mut g = self.entries.lock().expect("context store poisoned");
        g.remove(id).map(|(p, _)| p)
    }
}

pub type SharedContextStore = std::sync::Arc<ContextStore>;

/// One request from proxy → consumer task. The `KeyboardShortcut`
/// variant is the tab-webview-side keyboard relay: the init script
/// intercepts `Ctrl+…` shortcuts the shell already handles (T/W/L/F/
/// R/…) and forwards them so App.tsx's existing dispatcher can run,
/// even when the user is focused inside a page's content.
pub enum ContextMenuRequest {
    Open(ContextMenuPayload),
    Dismiss,
    /// `key` is the lowercase form of `KeyboardEvent.key`; `shift`
    /// is whether the shift modifier was held. Ctrl/meta are assumed
    /// (the init script only sends events where one was held).
    KeyboardShortcut {
        key: String,
        shift: bool,
    },
    /// Middle-click on a link. Opens `url` in a new BlueFlame tab.
    /// Matches Chrome/Firefox convention - desktop only; mobile has
    /// no middle button.
    OpenInNewTab {
        url: String,
    },
}

/// Per-launch auth token embedded into the init script. The proxy
/// sentinel handler rejects requests whose query param doesn't match,
/// so a malicious page can't forge a context-menu trigger by hand-
/// crafting its own sendBeacon. A fresh value every launch means
/// even a leaked token is short-lived. Generate once at boot and
/// hand an `Arc<String>` to every place that needs a read - the
/// string never changes, so no Mutex is needed.
pub type SharedContextToken = std::sync::Arc<String>;

/// Generate a fresh per-launch token value.
pub fn fresh_token() -> SharedContextToken {
    std::sync::Arc::new(uuid_v4_hex())
}

/// Read the current token string from Tauri state. Used by
/// `browser.rs` at tab-creation time to bake the value into the
/// injected init script.
pub fn current_token(app: &tauri::AppHandle) -> String {
    (**app.state::<SharedContextToken>()).clone()
}

/// Channel shape used by the proxy to push context-menu requests
/// into the main-thread consumer task spawned in lib.rs.
pub type ContextMenuTx = tokio::sync::mpsc::UnboundedSender<ContextMenuRequest>;
pub type ContextMenuRx = tokio::sync::mpsc::UnboundedReceiver<ContextMenuRequest>;
pub type SharedContextMenuTx = std::sync::Arc<std::sync::Mutex<Option<ContextMenuTx>>>;

/// Open the context-menu popup anchored to the click position. Called
/// from the consumer task spawned at boot in `lib.rs`; must run on
/// the main thread since it spawns a webview child.
pub async fn open_context_menu_popup(
    app: &tauri::AppHandle,
    payload: ContextMenuPayload,
) -> Result<(), String> {
    let main = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let (main_w, main_h) = {
        let size = main.inner_size().map_err(|e| format!("inner_size: {e}"))?;
        let scale = main.scale_factor().map_err(|e| format!("scale: {e}"))?;
        (size.width as f64 / scale, size.height as f64 / scale)
    };

    let panel_w = POPUP_WIDTH.min((main_w - 2.0 * POPUP_MARGIN).max(140.0));
    let panel_h = POPUP_MAX_HEIGHT.min((main_h - POPUP_MARGIN * 2.0).max(100.0));
    let panel_x = payload
        .screen_x
        .min(main_w - panel_w - POPUP_MARGIN)
        .max(POPUP_MARGIN);
    let panel_y = payload
        .screen_y
        .min(main_h - panel_h - POPUP_MARGIN)
        .max(POPUP_MARGIN);

    // Only one popup on screen at a time: close any existing context
    // popup AND the menu / trust popups before spawning.
    if let Some(wv) = app.get_webview(CONTEXT_MENU_LABEL) {
        let _ = wv.close();
    }
    if let Some(wv) = app.get_webview("menu-popup") {
        let _ = wv.close();
    }
    if let Some(wv) = app.get_webview("trust-panel") {
        let _ = wv.close();
    }

    let ctx_id = uuid_v4_hex();
    app.state::<SharedContextStore>()
        .put(ctx_id.clone(), payload);

    let base = if cfg!(debug_assertions) {
        "http://localhost:1420"
    } else {
        "http://tauri.localhost"
    };
    let mut popup_url = url::Url::parse(base).map_err(|e| format!("parse base url: {e}"))?;
    {
        let mut qp = popup_url.query_pairs_mut();
        qp.append_pair("panel", "context");
        qp.append_pair("ctx_id", &ctx_id);
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let app_clone = app.clone();
    let popup_url_clone = popup_url.clone();
    app.run_on_main_thread(move || {
        let res = app_clone
            .get_window("main")
            .ok_or_else(|| "main window not found".to_string())
            .and_then(|main| {
                main.add_child(
                    WebviewBuilder::new(CONTEXT_MENU_LABEL, WebviewUrl::External(popup_url_clone))
                        .background_color(POPUP_BG_COLOR),
                    LogicalPosition::new(panel_x, panel_y),
                    LogicalSize::new(panel_w, panel_h),
                )
                .map(|_| ())
                .map_err(|e| format!("add_child context menu: {e}"))
            });
        let _ = tx.send(res);
    })
    .map_err(|e| format!("run_on_main_thread: {e}"))?;
    rx.await
        .map_err(|e| format!("main-thread add_child dropped: {e}"))??;
    Ok(())
}

#[tauri::command]
pub fn close_context_menu(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(wv) = app.get_webview(CONTEXT_MENU_LABEL) {
        let _ = wv.close();
    }
    Ok(())
}

/// Shrink-to-fit the popup to the React component's measured height.
/// Pattern matches `resize_menu_popup`.
#[tauri::command]
pub fn resize_context_menu(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    if let Some(wv) = app.get_webview(CONTEXT_MENU_LABEL) {
        let main = app
            .get_window("main")
            .ok_or_else(|| "main window not found".to_string())?;
        let (_, main_h) = {
            let size = main.inner_size().map_err(|e| format!("inner_size: {e}"))?;
            let scale = main.scale_factor().map_err(|e| format!("scale: {e}"))?;
            (size.width as f64 / scale, size.height as f64 / scale)
        };
        let clamped = height.clamp(40.0, main_h - POPUP_MARGIN * 2.0);
        let _ = wv.set_size(LogicalSize::new(POPUP_WIDTH, clamped));
    }
    Ok(())
}

/// Frontend hands us the UUID from its URL; we return the payload
/// (and remove it from the store so a re-open with the same id
/// fails closed).
#[tauri::command]
pub fn get_context_payload(
    store: tauri::State<'_, SharedContextStore>,
    ctx_id: String,
) -> Result<ContextMenuPayload, String> {
    store
        .take(&ctx_id)
        .ok_or_else(|| "context payload not found or already consumed".to_string())
}

/// Tiny UUID v4 generator that avoids pulling in another crate just
/// for this. rand 0.9-style: grab 16 bytes of entropy, mask the
/// version + variant bits, hex-encode. Uses std::time::SystemTime +
/// a process-wide counter for uniqueness - not cryptographically
/// random, but this ID only has to be unique across concurrent popup
/// events within a single process, not unpredictable.
fn uuid_v4_hex() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;
    format!("{nanos:016x}{pid:08x}{seq:08x}")
}
