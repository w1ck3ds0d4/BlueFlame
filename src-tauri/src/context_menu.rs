//! Right-click / long-press context menu for pages loaded in tab
//! webviews.
//!
//! The popup webview is spawned ONCE at app boot ([`ensure_context_menu_popup`])
//! and parked offscreen at minimum size. Each right-click delivers a
//! fresh payload to that warm webview via the `context-menu:payload`
//! Tauri event ([`deliver_context_menu_payload`]); the React side
//! updates state, measures the rendered height, then calls
//! [`show_context_menu`] to position + reveal at the click coords.
//! Action clicks and dismiss both call [`hide_context_menu`] which
//! parks it offscreen again, so the React bundle stays warm for the
//! next event.
//!
//! Right-click events themselves arrive over the Tauri IPC bridge,
//! not HTTP. The previous implementation used a fake `blueflame.ipc`
//! host that the MITM proxy intercepted, but page CSP `connect-src`
//! directives silently dropped the request before it left the
//! webview. See `submit_tab_event` for the IPC entry point.

use serde::Serialize;
use tauri::{LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl};

/// Label of the context-menu popup webview. Unique so reposition,
/// resize, hide, and event-emit can all idempotently target it.
pub const CONTEXT_MENU_LABEL: &str = "context-menu";

/// Width of the popup. Height is computed from the rendered React
/// content per-event; this is the only fixed dimension.
const POPUP_WIDTH: f64 = 240.0;
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

/// Per-launch auth token embedded into the init script. The
/// `submit_tab_event` IPC command rejects events whose token doesn't
/// match, so a hostile page that monkey-patches the IPC bridge can't
/// fabricate events without first scraping the (function-scoped)
/// token out of the init script. A fresh value every launch means
/// even a leaked token is short-lived. Generate once at boot and
/// hand an `Arc<String>` to every place that needs a read - the
/// string never changes, so no Mutex is needed.
pub type SharedContextToken = std::sync::Arc<String>;

/// Generate a fresh per-launch token value.
pub fn fresh_token() -> SharedContextToken {
    std::sync::Arc::new(token_hex())
}

/// 32-hex-char random-ish string for the per-launch token.
fn token_hex() -> String {
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

/// JSON payload posted by the per-tab init script via Tauri IPC. The
/// `kind` discriminator picks which `ContextMenuRequest` variant to
/// dispatch. We can't use the request enum directly because serde's
/// untagged enum deserialization is fragile across struct variants;
/// the explicit tag keeps the wire format predictable.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TabEvent {
    Open {
        page: String,
        x: f64,
        y: f64,
        #[serde(default)]
        link: Option<String>,
        #[serde(default, rename = "linkText")]
        link_text: Option<String>,
        #[serde(default)]
        image: Option<String>,
        #[serde(default)]
        sel: Option<String>,
    },
    Dismiss,
    Kbd {
        key: String,
        #[serde(default)]
        shift: bool,
    },
    Middleclick {
        url: String,
    },
}

/// Tauri IPC entry point invoked by the per-tab init script for every
/// right-click, middle-click, dismiss, and keyboard relay event. We
/// dropped the previous proxy-based HTTP sentinel because page CSP
/// `connect-src` directives blocked the request before the MITM proxy
/// ever saw it (sendBeacon returned true but the browser silently
/// dropped the network call). IPC bypasses CSP entirely because it
/// rides Tauri's postMessage bridge, not a network request.
///
/// Token check is preserved so a hostile page that monkey-patches
/// `__TAURI_INTERNALS__.invoke` can't fabricate events without first
/// scraping the token out of the (function-scoped) init script.
#[tauri::command]
pub fn submit_tab_event(
    tx_state: tauri::State<'_, SharedContextMenuTx>,
    token_state: tauri::State<'_, SharedContextToken>,
    token: String,
    event: TabEvent,
) -> Result<(), String> {
    let expected: &str = &token_state;
    if token.len() != expected.len() {
        return Err("token length mismatch".into());
    }
    let mut diff = 0u8;
    for (a, b) in expected.as_bytes().iter().zip(token.as_bytes()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return Err("token mismatch".into());
    }

    let request = match event {
        TabEvent::Open {
            page,
            x,
            y,
            link,
            link_text,
            image,
            sel,
        } => ContextMenuRequest::Open(ContextMenuPayload {
            page_url: page,
            link_url: link.filter(|s| !s.is_empty()),
            link_text: link_text.filter(|s| !s.is_empty()),
            image_url: image.filter(|s| !s.is_empty()),
            selection_text: sel.filter(|s| !s.is_empty()),
            screen_x: x,
            screen_y: y,
        }),
        TabEvent::Dismiss => ContextMenuRequest::Dismiss,
        TabEvent::Kbd { key, shift } => {
            if key.is_empty() {
                return Err("kbd event missing key".into());
            }
            ContextMenuRequest::KeyboardShortcut { key, shift }
        }
        TabEvent::Middleclick { url } => {
            if url.is_empty() {
                return Err("middleclick event missing url".into());
            }
            ContextMenuRequest::OpenInNewTab { url }
        }
    };

    if let Ok(g) = tx_state.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(request);
        }
    }
    Ok(())
}

/// Park the popup webview offscreen at minimum size. Used as the
/// "hidden" state - cheaper than create/destroy on every right-click,
/// and lets us keep the React bundle warm so subsequent RMBs feel
/// instant. Some platforms ignore set_size(0,0), so 1x1 with a far-
/// negative position is the safe portable hide.
const HIDDEN_POSITION: LogicalPosition<f64> = LogicalPosition::new(-9999.0, -9999.0);
const HIDDEN_SIZE: LogicalSize<f64> = LogicalSize::new(1.0, 1.0);

/// Spawn the context-menu popup webview parked offscreen. Called once
/// at app boot so right-click can later just reposition + resize a
/// pre-warmed webview instead of paying the ~150-300 ms WebView2
/// process-spawn + React-bundle-load cost on every click. Idempotent:
/// if the webview already exists (e.g. consumer task re-runs), no-op.
pub async fn ensure_context_menu_popup(app: &tauri::AppHandle) -> Result<(), String> {
    if app.get_webview(CONTEXT_MENU_LABEL).is_some() {
        return Ok(());
    }

    let base = if cfg!(debug_assertions) {
        "http://localhost:1420"
    } else {
        "http://tauri.localhost"
    };
    let mut popup_url = url::Url::parse(base).map_err(|e| format!("parse base url: {e}"))?;
    popup_url.query_pairs_mut().append_pair("panel", "context");

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let app_clone = app.clone();
    app.run_on_main_thread(move || {
        let res = app_clone
            .get_window("main")
            .ok_or_else(|| "main window not found".to_string())
            .and_then(|main| {
                main.add_child(
                    WebviewBuilder::new(CONTEXT_MENU_LABEL, WebviewUrl::External(popup_url))
                        .background_color(POPUP_BG_COLOR),
                    HIDDEN_POSITION,
                    HIDDEN_SIZE,
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

/// Send a fresh payload to the warm popup. The React side listens for
/// `context-menu:payload`, updates its state, measures, and then calls
/// [`show_context_menu`] to position + reveal. We hide first so that
/// if the popup happens to already be visible (rapid double-RMB) the
/// user doesn't briefly see the old menu at the new position.
pub fn deliver_context_menu_payload(
    app: &tauri::AppHandle,
    payload: ContextMenuPayload,
) -> Result<(), String> {
    use tauri::Emitter;

    if let Some(wv) = app.get_webview(CONTEXT_MENU_LABEL) {
        let _ = wv.set_position(HIDDEN_POSITION);
        let _ = wv.set_size(HIDDEN_SIZE);
    }
    // Close the sibling popups (hamburger, trust) so the context menu
    // is the only popup on screen when it appears.
    if let Some(wv) = app.get_webview("menu-popup") {
        let _ = wv.close();
    }
    if let Some(wv) = app.get_webview("trust-panel") {
        let _ = wv.close();
    }

    app.emit_to(CONTEXT_MENU_LABEL, "context-menu:payload", payload)
        .map_err(|e| format!("emit context-menu:payload: {e}"))?;
    Ok(())
}

/// Position + size the popup so it's visible at the click coords. The
/// React side calls this from `useLayoutEffect` once it has measured
/// its rendered height, so by the time the popup leaves its hidden
/// parking spot the contents are already laid out at the right
/// dimensions - no flicker, no shrink-to-fit pop.
#[tauri::command]
pub fn show_context_menu(app: tauri::AppHandle, x: f64, y: f64, height: f64) -> Result<(), String> {
    let Some(wv) = app.get_webview(CONTEXT_MENU_LABEL) else {
        return Ok(());
    };
    let main = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let (main_w, main_h) = {
        let size = main.inner_size().map_err(|e| format!("inner_size: {e}"))?;
        let scale = main.scale_factor().map_err(|e| format!("scale: {e}"))?;
        (size.width as f64 / scale, size.height as f64 / scale)
    };

    let panel_w = POPUP_WIDTH.min((main_w - 2.0 * POPUP_MARGIN).max(140.0));
    let panel_h = height.clamp(40.0, (main_h - POPUP_MARGIN * 2.0).max(100.0));
    let panel_x = x.min(main_w - panel_w - POPUP_MARGIN).max(POPUP_MARGIN);
    let panel_y = y.min(main_h - panel_h - POPUP_MARGIN).max(POPUP_MARGIN);

    let _ = wv.set_size(LogicalSize::new(panel_w, panel_h));
    let _ = wv.set_position(LogicalPosition::new(panel_x, panel_y));
    Ok(())
}

/// Hide the popup by parking it offscreen at minimum size. Replaces
/// the previous close-the-webview behavior so subsequent RMBs reuse
/// the same warm webview instead of spawning a fresh one.
#[tauri::command]
pub fn hide_context_menu(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(wv) = app.get_webview(CONTEXT_MENU_LABEL) {
        let _ = wv.set_position(HIDDEN_POSITION);
        let _ = wv.set_size(HIDDEN_SIZE);
    }
    Ok(())
}

/// Legacy command name kept so frontend code that still calls
/// `close_context_menu` keeps working through the migration. The
/// React component itself was updated to call `hide_context_menu`,
/// but anything that escaped the rename will still hit this and do
/// the right thing.
#[tauri::command]
pub fn close_context_menu(app: tauri::AppHandle) -> Result<(), String> {
    hide_context_menu(app)
}
