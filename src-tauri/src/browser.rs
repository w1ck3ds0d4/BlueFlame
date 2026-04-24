//! Browser chrome with tabs.
//!
//! Each tab owns a native child webview labeled `browse-<id>`. The React
//! shell renders the URL bar and tab strip on top; the active tab's webview
//! fills the content area below the chrome. Inactive tabs stay alive but
//! are sized to zero so they keep their DOM/scroll state on switch without
//! covering the view.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl};

use crate::metasearch;
use crate::new_tab;
use crate::search::{SearchEngine, SearchSettings};

/// Vertical offset reserved for the shell chrome on desktop: custom
/// titlebar (30px) + URL bar row + tab strip + bookmarks bar. Must
/// match the CSS in the React app. The old `brand-row` is gone; its
/// contents moved into the sidebar.
const CHROME_HEIGHT: f64 = 134.0;
/// Mobile chrome is a single 56px action row under the 30px titlebar -
/// no separate tab strip or bookmarks bar. Tabs live in the overlay
/// switcher; bookmarks are in the kebab menu.
const MOBILE_CHROME_HEIGHT: f64 = 30.0 + 56.0;
/// Horizontal offset reserved for the left activity sidebar (brand + nav +
/// proxy status). Webview children are positioned after it.
const SIDEBAR_WIDTH: f64 = 48.0;

/// User-agent the browser sends when the user has opted into mobile
/// mode via Settings. Modern Pixel-on-Android Chrome string - picked
/// because the big UA-sniffing sites (YouTube, Reddit, Twitter) all
/// match "Android" + "Mobile" in their detection logic and serve the
/// mobile layout when they see it.
const MOBILE_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36";

/// Injected into every mobile-mode tab BEFORE any page script runs.
/// UA spoofing alone isn't enough for sites like m.youtube.com - they
/// run client-side JS that re-checks "am I really on mobile?" via
/// several signals, and if ANY of them looks desktop they bounce
/// back to the desktop site. This script lies about all of them.
///
/// Signals covered:
/// - navigator.maxTouchPoints + window.ontouchstart (touch feature)
/// - @media (hover: none) / (pointer: coarse) (input modality)
/// - navigator.userAgentData.mobile + brands (modern Client Hints API)
/// - navigator.platform (legacy Android detection)
/// - screen.width / screen.height (phone-sized physical display)
/// - window.devicePixelRatio (retina-ish density)
///
/// Viewport width (window.innerWidth) is NOT spoofed - it reflects
/// the real webview size and we don't want to lie to CSS media
/// queries that drive the page's own responsive layout.
/// JS injected into every tab webview so right-click / long-press
/// surfaces the BlueFlame context menu instead of the native one.
/// The handler captures target info (link, image, selection, page
/// URL) + click coordinates, then fires a POST to the sentinel host
/// `blueflame.ipc` via `navigator.sendBeacon`. The MITM proxy matches
/// that host and emits a Rust-side request to open the popup.
///
/// The per-launch token prevents a malicious page from forging its
/// own context-menu requests: only the injected script knows the
/// token. The template replaces `__BF_TOKEN__` at webview-creation
/// time so each BlueFlame run gets a fresh value.
///
/// Long-press threshold: 500 ms, matching Android/iOS convention.
/// Scroll (touchmove beyond ~10 px) cancels the press so legitimate
/// swipes don't trigger a menu.
const CONTEXT_MENU_INIT_SCRIPT_TEMPLATE: &str = r#"
(function () {
    var BF_TOKEN = "__BF_TOKEN__";
    var SENTINEL = "http://blueflame.ipc/context";
    var LONG_PRESS_MS = 500;
    var MOVE_CANCEL_PX = 10;

    function climbForLink(el) {
        while (el && el !== document.documentElement) {
            if (el.tagName === "A" && el.href) return el;
            el = el.parentElement;
        }
        return null;
    }
    function climbForImage(el) {
        while (el && el !== document.documentElement) {
            if (el.tagName === "IMG" && el.src) return el;
            el = el.parentElement;
        }
        return null;
    }
    function selectionText() {
        try {
            var s = window.getSelection && window.getSelection();
            return s ? s.toString() : "";
        } catch (_) {
            return "";
        }
    }

    function send(ctx) {
        try {
            var qs = new URLSearchParams();
            qs.set("token", BF_TOKEN);
            qs.set("page", location.href);
            qs.set("x", String(Math.round(ctx.x)));
            qs.set("y", String(Math.round(ctx.y)));
            if (ctx.link) qs.set("link", ctx.link);
            if (ctx.linkText) qs.set("linkText", ctx.linkText);
            if (ctx.image) qs.set("image", ctx.image);
            if (ctx.sel) qs.set("sel", ctx.sel);
            var url = SENTINEL + "?" + qs.toString();
            if (navigator.sendBeacon) {
                // Empty body with the sentinel path + query string is
                // enough: the proxy reads everything out of the URL.
                navigator.sendBeacon(url);
            } else {
                // Fallback: fire-and-forget POST. Some webviews
                // disable sendBeacon; fetch with keepalive gives us
                // the same semantics.
                fetch(url, { method: "POST", keepalive: true }).catch(function () {});
            }
        } catch (_) { /* swallow - never break the page */ }
    }

    function build(target, clientX, clientY) {
        var link = climbForLink(target);
        var img = climbForImage(target);
        var ctx = {
            x: clientX,
            y: clientY,
            link: link ? link.href : "",
            linkText: link ? (link.textContent || "").trim().slice(0, 120) : "",
            image: img ? img.src : "",
            sel: selectionText().slice(0, 400),
        };
        return ctx;
    }

    // Dismiss any open context popup by firing a sentinel with a
    // `dismiss=1` flag. Cheap to send; the proxy handler recognizes
    // `dismiss` and routes to the close path instead of opening a
    // popup. Tracking `menuOpen` locally keeps this from being sent
    // on every click - only after we've actually shown a menu.
    var menuOpen = false;
    function dismissSentinel() {
        if (!menuOpen) return;
        menuOpen = false;
        try {
            var qs = new URLSearchParams();
            qs.set("token", BF_TOKEN);
            qs.set("dismiss", "1");
            var url = SENTINEL + "?" + qs.toString();
            if (navigator.sendBeacon) navigator.sendBeacon(url);
            else fetch(url, { method: "POST", keepalive: true }).catch(function () {});
        } catch (_) { /* ignore */ }
    }

    // Desktop right-click + most webviews' long-press → contextmenu.
    window.addEventListener("contextmenu", function (e) {
        try {
            e.preventDefault();
            send(build(e.target, e.clientX, e.clientY));
            menuOpen = true;
        } catch (_) { /* ignore */ }
    }, true);
    window.addEventListener("click", dismissSentinel, true);
    window.addEventListener("keydown", function (e) {
        if (e.key === "Escape") dismissSentinel();
    }, true);

    // iOS/Android long-press: contextmenu is often suppressed by the
    // native gesture recognizer, so time the touch ourselves.
    var touchState = null;
    window.addEventListener("touchstart", function (e) {
        if (!e.touches || e.touches.length !== 1) return;
        var t = e.touches[0];
        touchState = {
            x: t.clientX,
            y: t.clientY,
            target: e.target,
            timer: window.setTimeout(function () {
                if (!touchState) return;
                try {
                    send(build(touchState.target, touchState.x, touchState.y));
                    menuOpen = true;
                } catch (_) { /* ignore */ }
                touchState = null;
            }, LONG_PRESS_MS),
        };
    }, { passive: true, capture: true });
    function cancelTouch() {
        if (touchState) {
            window.clearTimeout(touchState.timer);
            touchState = null;
        }
    }
    window.addEventListener("touchend", cancelTouch, { passive: true, capture: true });
    window.addEventListener("touchcancel", cancelTouch, { passive: true, capture: true });
    window.addEventListener("touchmove", function (e) {
        if (!touchState || !e.touches || e.touches.length !== 1) return;
        var t = e.touches[0];
        var dx = t.clientX - touchState.x;
        var dy = t.clientY - touchState.y;
        if (Math.hypot(dx, dy) > MOVE_CANCEL_PX) cancelTouch();
    }, { passive: true, capture: true });
})();
"#;

const MOBILE_INIT_SCRIPT: &str = r#"
(function () {
    try {
        // --- touch + pointer signals ---
        Object.defineProperty(navigator, 'maxTouchPoints', {
            get: () => 5,
            configurable: true,
        });
        if (!('ontouchstart' in window)) {
            Object.defineProperty(window, 'ontouchstart', {
                value: null,
                configurable: true,
            });
        }
        var origMM = window.matchMedia.bind(window);
        var mqNoop = {
            matches: true,
            media: '',
            addListener: function () {},
            removeListener: function () {},
            addEventListener: function () {},
            removeEventListener: function () {},
            onchange: null,
            dispatchEvent: function () { return false; },
        };
        window.matchMedia = function (q) {
            if (/\(\s*(any-)?(hover)\s*:\s*none\s*\)/i.test(q)) {
                return Object.assign({}, mqNoop, { media: q });
            }
            if (/\(\s*(any-)?(pointer)\s*:\s*coarse\s*\)/i.test(q)) {
                return Object.assign({}, mqNoop, { media: q });
            }
            return origMM(q);
        };

        // --- User-Agent Client Hints (structured UA) ---
        // Modern sites read navigator.userAgentData.mobile instead of
        // sniffing the UA string. Chrome on Android sets this to true;
        // desktop Chrome sets false. Override with a full mock.
        var brands = [
            { brand: 'Chromium', version: '131' },
            { brand: 'Not_A Brand', version: '24' },
            { brand: 'Google Chrome', version: '131' },
        ];
        Object.defineProperty(navigator, 'userAgentData', {
            get: () => ({
                mobile: true,
                brands: brands,
                platform: 'Android',
                getHighEntropyValues: function (hints) {
                    return Promise.resolve({
                        architecture: '',
                        bitness: '',
                        brands: brands,
                        fullVersionList: brands.map(function (b) {
                            return { brand: b.brand, version: b.version + '.0.0.0' };
                        }),
                        mobile: true,
                        model: 'Pixel 8',
                        platform: 'Android',
                        platformVersion: '14.0.0',
                        uaFullVersion: '131.0.0.0',
                        wow64: false,
                    });
                },
                toJSON: function () {
                    return { mobile: true, brands: brands, platform: 'Android' };
                },
            }),
            configurable: true,
        });

        // --- platform + vendor ---
        Object.defineProperty(navigator, 'platform', {
            get: () => 'Linux armv81',
            configurable: true,
        });

        // --- screen dimensions: phone-sized physical display ---
        Object.defineProperty(screen, 'width', { get: () => 412, configurable: true });
        Object.defineProperty(screen, 'height', { get: () => 915, configurable: true });
        Object.defineProperty(screen, 'availWidth', { get: () => 412, configurable: true });
        Object.defineProperty(screen, 'availHeight', { get: () => 915, configurable: true });

        // --- pixel density: phones typically report ~2.6-3.0 ---
        Object.defineProperty(window, 'devicePixelRatio', {
            get: () => 3,
            configurable: true,
        });
    } catch (e) {
        // Never let the spoof script break the page.
    }
})();
"#;

fn tab_label(id: u64) -> String {
    format!("browse-{id}")
}

/// Metadata shown to the UI. Title is a host-derived best guess until we wire
/// up navigation events in a follow-up.
#[derive(Debug, Clone, Serialize)]
pub struct TabInfo {
    pub id: u64,
    pub url: String,
    pub title: String,
    /// `true` while the webview is navigating or loading the current page.
    /// The tab strip renders a braille spinner instead of the favicon in
    /// this state.
    pub loading: bool,
    /// `true` for tabs opened in private mode. Pages visited in a private
    /// tab are not recorded in the personal-index history and the tab is
    /// not persisted to the session file on close. Visual: the tab picks
    /// up a warn-colored accent.
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Default)]
pub struct TabsState {
    next_id: u64,
    active_id: Option<u64>,
    tabs: Vec<TabInfo>,
}

impl TabsState {
    fn issue_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Id of the tab at position `idx` (the order they were opened in).
    /// Used by session-restore to re-activate the right tab after all
    /// saved URLs have been reopened.
    pub fn id_at(&self, idx: usize) -> Option<u64> {
        self.tabs.get(idx).map(|t| t.id)
    }
}

pub type Tabs = Mutex<TabsState>;

#[derive(Debug, Clone, Serialize)]
pub struct TabsView {
    pub tabs: Vec<TabInfo>,
    pub active_id: Option<u64>,
}

#[tauri::command]
pub async fn browser_new_tab(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
) -> Result<TabsView, String> {
    tracing::info!("browser_new_tab");
    let engine = current_engine(&app);
    let tiles = collect_speed_dial_tiles(&app, 8);
    let url = new_tab::data_url(engine, &tiles);
    let result = open_tab_impl(app, tabs, url, false).await;
    if let Err(e) = &result {
        tracing::error!(error = %e, "browser_new_tab failed");
    }
    result
}

/// Open a fresh tab in private mode - identical to browser_new_tab except
/// that visits won't be recorded in history and the tab won't be written
/// to session.json.
#[tauri::command]
pub async fn browser_new_private_tab(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
) -> Result<TabsView, String> {
    tracing::info!("browser_new_private_tab");
    let engine = current_engine(&app);
    // Private tabs skip the speed-dial tiles (which leak recent history)
    // and land on a fresh new-tab page with no quick-access list.
    let url = new_tab::data_url(engine, &[]);
    let result = open_tab_impl(app, tabs, url, true).await;
    if let Err(e) = &result {
        tracing::error!(error = %e, "browser_new_private_tab failed");
    }
    result
}

/// Top-visited history merged with bookmarks, de-duplicated by URL.
/// Bookmarks always win when the same URL shows up in both lists.
fn collect_speed_dial_tiles(app: &tauri::AppHandle, limit: usize) -> Vec<new_tab::Tile> {
    let Some(store) = app.try_state::<crate::storage::SharedStore>() else {
        return Vec::new();
    };
    let Ok(guard) = store.lock() else {
        return Vec::new();
    };
    let bookmarks = guard.list_bookmarks().unwrap_or_default();
    let history = guard.top_visited(limit * 2).unwrap_or_default();

    let mut out: Vec<new_tab::Tile> = Vec::with_capacity(limit);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for b in bookmarks.into_iter().take(limit) {
        if seen.insert(b.url.clone()) {
            out.push(new_tab::Tile {
                title: b.title,
                url: b.url,
                source: new_tab::TileSource::Bookmark,
            });
        }
    }
    for v in history {
        if out.len() >= limit {
            break;
        }
        if seen.insert(v.url.clone()) {
            out.push(new_tab::Tile {
                title: v.title,
                url: v.url,
                source: new_tab::TileSource::History,
            });
        }
    }
    out
}

#[tauri::command]
pub fn browser_list_tabs(tabs: tauri::State<'_, Tabs>) -> Result<TabsView, String> {
    let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
    Ok(TabsView {
        tabs: s.tabs.clone(),
        active_id: s.active_id,
    })
}

#[tauri::command]
pub async fn browser_open_tab(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
    url: String,
) -> Result<TabsView, String> {
    open_tab_impl(app, tabs, url, false).await
}

async fn open_tab_impl(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
    url: String,
    is_private: bool,
) -> Result<TabsView, String> {
    let engine = current_engine(&app);
    let target = resolve_url(&url, engine).map_err(|e| format!("invalid url: {e}"))?;
    let parsed: url::Url = target.parse().map_err(|e| format!("parse url: {e}"))?;
    let title = host_title(&parsed);
    let mobile_ua = mobile_ua_enabled(&app);
    tracing::info!(target = %trace_target(&target), "browser_open_tab start");

    tracing::info!("browser_open_tab: resolving main window");
    let main = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let chrome_h = if mobile_ua {
        MOBILE_CHROME_HEIGHT
    } else {
        CHROME_HEIGHT
    };
    let sidebar_w = if mobile_ua { 0.0 } else { SIDEBAR_WIDTH };
    let (w, h) = content_size_with(&main, sidebar_w, chrome_h)?;
    tracing::info!(w, h, "browser_open_tab: content size");

    let prev_active = {
        let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        s.active_id
    };
    if let Some(prev) = prev_active {
        if let Some(wv) = app.get_webview(&tab_label(prev)) {
            if let Err(e) = wv.set_size(LogicalSize::new(0.0, 0.0)) {
                tracing::warn!(prev, error = %e, "failed to shrink previous tab");
            }
        }
    }
    tracing::info!(?prev_active, "browser_open_tab: prev shrunk");

    let id = {
        let mut s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        s.issue_id()
    };
    let label = tab_label(id);
    tracing::info!(id, label = %label, "browser_open_tab: scheduling add_child on main thread");

    // WebView2 on Windows requires add_child to run on the main thread
    // (the one that owns the parent window). Tauri commands run on a
    // worker thread, so dispatch the call via run_on_main_thread and
    // await the result over a oneshot. Without this the second tab's
    // add_child hangs forever because the message never gets pumped.
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let app_clone = app.clone();
    let label_clone = label.clone();
    let parsed_clone = parsed.clone();
    let nav_app = app.clone();
    let tab_id_for_nav = id;
    let load_app = app.clone();
    app.run_on_main_thread(move || {
        // Fires on every navigation: redirects, in-page links, SPA route
        // swaps. Use it to keep TabInfo.url / title in sync with the page
        // the user is actually on, flip the tab into its loading state,
        // and kick off a favicon fetch for the new host.
        let on_navigation = move |u: &url::Url| -> bool {
            let url_str = u.to_string();
            let host_opt = u.host_str().map(|s| s.to_string());
            let derived_title = host_title(u);
            let app = nav_app.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(state) = app.try_state::<Tabs>() {
                    if let Ok(mut s) = state.lock() {
                        if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab_id_for_nav) {
                            if !url_str.starts_with("data:") && !url_str.starts_with("about:") {
                                t.url = url_str.clone();
                                t.title = derived_title;
                                t.loading = true;
                            }
                        }
                    }
                }
                let _ = app.emit("blueflame:tabs-changed", ());
                persist_session(&app);

                if let Some(host) = host_opt {
                    maybe_fetch_favicon(&app, host).await;
                }
            });
            true
        };

        // Fires when a page's DOM has finished loading. Flip loading off.
        let load_app_cb = load_app.clone();
        let on_page_load =
            move |_wv: tauri::Webview, payload: tauri::webview::PageLoadPayload<'_>| {
                let event = payload.event();
                let app = load_app_cb.clone();
                // Only react on the 'finished' side; 'started' already fired on
                // on_navigation above.
                if !matches!(event, tauri::webview::PageLoadEvent::Finished) {
                    return;
                }
                tauri::async_runtime::spawn(async move {
                    if let Some(state) = app.try_state::<Tabs>() {
                        if let Ok(mut s) = state.lock() {
                            if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab_id_for_nav) {
                                t.loading = false;
                            }
                        }
                    }
                    let _ = app.emit("blueflame:tabs-changed", ());
                    persist_session(&app);
                });
            };

        let res = app_clone
            .get_window("main")
            .ok_or_else(|| "main window not found".to_string())
            .and_then(|main| {
                // Private tabs get an ephemeral browsing profile so
                // cookies, localStorage, and IndexedDB don't persist
                // between sessions AND don't leak back into regular
                // tabs. WebView2 on Windows honors this as InPrivate
                // mode; Linux/macOS use their platform equivalent.
                let mut b = WebviewBuilder::new(label_clone, WebviewUrl::External(parsed_clone))
                    .on_navigation(on_navigation)
                    .on_page_load(on_page_load);
                if is_private {
                    b = b.incognito(true);
                }
                if mobile_ua {
                    b = b.user_agent(MOBILE_USER_AGENT);
                    b = b.initialization_script(MOBILE_INIT_SCRIPT);
                }
                // Context-menu bridge runs on every tab regardless of
                // mobile/desktop mode: the handler swallows right-click
                // AND long-press events + posts the target metadata to
                // the `blueflame.ipc` sentinel host, which the proxy
                // intercepts and turns into a popup. Token is
                // per-launch so a malicious page can't forge requests.
                let token = crate::context_menu::current_token(&app_clone);
                let ctx_script = CONTEXT_MENU_INIT_SCRIPT_TEMPLATE.replace("__BF_TOKEN__", &token);
                b = b.initialization_script(ctx_script);
                // macOS: route tab traffic through the local MITM
                // proxy via per-webview proxy_url. Windows + Linux
                // pick this up from the env vars set before any
                // webview spawn (see `configure_webview_proxy`).
                #[cfg(target_os = "macos")]
                {
                    let proxy = format!("http://127.0.0.1:{}", crate::PROXY_PORT);
                    if let Ok(u) = url::Url::parse(&proxy) {
                        b = b.proxy_url(u);
                    }
                }
                // Mobile mode centers a phone-sized rectangle; desktop
                // fills the full content area below the chrome.
                let (pos, size) = active_tab_bounds(&app_clone);
                main.add_child(b, pos, size)
                    .map(|_| ())
                    .map_err(|e| format!("add_child webview: {e}"))
            });
        let _ = tx.send(res);
    })
    .map_err(|e| format!("run_on_main_thread: {e}"))?;

    rx.await
        .map_err(|e| format!("main-thread add_child dropped: {e}"))?
        .map_err(|e| {
            tracing::error!(error = %e, id, "add_child webview failed");
            e
        })?;
    tracing::info!(id, "browser_open_tab: webview created");

    {
        let mut s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        s.tabs.push(TabInfo {
            id,
            url: target.clone(),
            title: title.clone(),
            loading: !target.starts_with("data:"),
            private: is_private,
        });
        s.active_id = Some(id);
    }

    if !is_private {
        record_visit(&app, &target, &title);
    }

    let view = {
        let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        TabsView {
            tabs: s.tabs.clone(),
            active_id: s.active_id,
        }
    };
    persist_session(&app);
    tracing::info!(tabs = view.tabs.len(), "browser_open_tab done");
    Ok(view)
}

/// Shorten long URLs (chiefly `data:` URLs for the new-tab page) so a single
/// `browser_open_tab start` log entry doesn't flood the terminal with the
/// entire base64-encoded HTML.
fn trace_target(url: &str) -> String {
    const MAX: usize = 80;
    if url.len() <= MAX {
        url.to_string()
    } else if let Some(i) = url.find(';').or_else(|| url.find(':')) {
        format!("{}... ({} bytes)", &url[..i.min(MAX)], url.len())
    } else {
        format!("{}... ({} bytes)", &url[..MAX], url.len())
    }
}

#[tauri::command]
pub fn browser_switch_tab(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
    id: u64,
) -> Result<TabsView, String> {
    let mut s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
    if !s.tabs.iter().any(|t| t.id == id) {
        return Err(format!("unknown tab id {id}"));
    }

    // New active tab gets the mobile/desktop-aware bounds; everyone
    // else shrinks to 0x0 so inactive tabs don't paint over the
    // visible one.
    let (active_pos, active_size) = active_tab_bounds(&app);
    for t in &s.tabs {
        if let Some(wv) = app.get_webview(&tab_label(t.id)) {
            if t.id == id {
                let _ = wv.set_position(active_pos);
                let _ = wv.set_size(active_size);
            } else {
                let _ = wv.set_size(LogicalSize::new(0.0, 0.0));
            }
        }
    }
    s.active_id = Some(id);

    let view = TabsView {
        tabs: s.tabs.clone(),
        active_id: s.active_id,
    };
    drop(s);
    persist_session(&app);
    Ok(view)
}

/// Close every existing tab and reopen it with the currently-
/// configured UA / privacy setting. Called from `set_mobile_ua` so
/// flipping the toggle feels "global" - WKWebView / WebView2 can't
/// swap UA on a live webview, so destroy + recreate is the only path.
///
/// Preserves: URL per tab, order, private/regular split, and which
/// tab was active. Does NOT preserve: scroll position, form input,
/// and private-tab session state (incognito is ephemeral by design,
/// so a rebuilt private tab is logged out).
pub async fn rebuild_all_tabs(app: &tauri::AppHandle) -> Result<(), String> {
    let tabs = app.state::<Tabs>();

    // Snapshot everything we need before mutating.
    let (snapshot, active_idx) = {
        let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        if s.tabs.is_empty() {
            return Ok(());
        }
        let snap: Vec<(String, bool)> = s.tabs.iter().map(|t| (t.url.clone(), t.private)).collect();
        let idx = s
            .active_id
            .and_then(|id| s.tabs.iter().position(|t| t.id == id));
        (snap, idx)
    };

    // Close every existing webview, then clear TabsState. We keep the
    // `next_id` counter monotonically increasing so new tabs get fresh
    // labels and we don't race a yet-to-be-finalized close against an
    // add_child that wants to reuse the old one.
    let ids_to_close: Vec<u64> = {
        let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        s.tabs.iter().map(|t| t.id).collect()
    };
    for id in ids_to_close {
        if let Some(wv) = app.get_webview(&tab_label(id)) {
            let _ = wv.close();
        }
    }
    {
        let mut s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        s.tabs.clear();
        s.active_id = None;
    }

    // Reopen each tab sequentially. open_tab_impl's add_child must run
    // on the main thread and races with concurrent opens (see #35), so
    // await each one before starting the next.
    for (url, private) in &snapshot {
        let ts = app.state::<Tabs>();
        if let Err(e) = open_tab_impl(app.clone(), ts, url.clone(), *private).await {
            tracing::warn!(url = %url, error = %e, "rebuild: open_tab_impl failed");
        }
    }

    // Restore active tab by index, since the id list has been re-
    // issued and the old ids are gone.
    if let Some(idx) = active_idx {
        let target_id = {
            let ts = app.state::<Tabs>();
            let s = ts.lock().map_err(|e| format!("lock tabs: {e}"))?;
            s.tabs.get(idx).map(|t| t.id)
        };
        if let Some(id) = target_id {
            let ts = app.state::<Tabs>();
            let _ = browser_switch_tab(app.clone(), ts, id);
        }
    }

    let _ = app.emit("blueflame:tabs-changed", ());
    Ok(())
}

#[tauri::command]
pub fn browser_close_tab(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
    id: u64,
) -> Result<TabsView, String> {
    if let Some(wv) = app.get_webview(&tab_label(id)) {
        let _ = wv.close();
    }

    let mut s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
    s.tabs.retain(|t| t.id != id);

    // If we closed the active tab, promote the last remaining one
    // and resize it via the shared bounds helper (desktop / mobile
    // aware).
    if s.active_id == Some(id) {
        s.active_id = s.tabs.last().map(|t| t.id);
        if let Some(new_active) = s.active_id {
            if let Some(wv) = app.get_webview(&tab_label(new_active)) {
                let (pos, size) = active_tab_bounds(&app);
                let _ = wv.set_position(pos);
                let _ = wv.set_size(size);
            }
        }
    }

    let view = TabsView {
        tabs: s.tabs.clone(),
        active_id: s.active_id,
    };
    drop(s);
    persist_session(&app);
    Ok(view)
}

#[tauri::command]
pub async fn browser_navigate_active(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
    url: String,
) -> Result<TabsView, String> {
    tracing::info!(input = %url, "browser_navigate_active");
    let engine = current_engine(&app);
    let meta_on = metasearch_enabled(&app);

    // Metasearch short-circuit: when the input is a plain query and the user
    // opted in, render BlueFlame's own results page.
    let target = if meta_on && looks_like_search_query(&url) {
        let results = metasearch::search(url.trim(), 20).await.unwrap_or_default();
        metasearch::results_data_url(url.trim(), &results)
    } else {
        resolve_url(&url, engine).map_err(|e| {
            tracing::error!(error = %e, "resolve_url failed");
            format!("invalid url: {e}")
        })?
    };
    let parsed: url::Url = target.parse().map_err(|e| {
        tracing::error!(error = %e, target_url = %target, "parse url failed");
        format!("parse url: {e}")
    })?;
    let title = if target.starts_with("data:") && meta_on {
        format!("Search: {}", url.trim())
    } else {
        host_title(&parsed)
    };
    tracing::info!(target_url = %target, "navigating active tab");

    // Snapshot the active tab id under a tight lock, then call wv.navigate
    // WITHOUT holding the lock - navigate can fire on_navigation callbacks
    // whose spawn tries to take this same lock; holding across the call
    // means we're racing ourselves.
    let active_id = {
        let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        s.active_id
    };

    let Some(id) = active_id else {
        tracing::warn!("no active tab - opening a new one for this navigation");
        return browser_open_tab(app, tabs, url).await;
    };

    let Some(wv) = app.get_webview(&tab_label(id)) else {
        tracing::warn!(
            active_id = id,
            "active tab has no child webview - falling through to open a new one"
        );
        return browser_open_tab(app, tabs, url).await;
    };

    wv.navigate(parsed).map_err(|e| {
        tracing::error!(error = %e, "webview.navigate failed");
        format!("navigate: {e}")
    })?;

    // Now update our cached TabInfo and return a fresh view under a second
    // lock. The on_navigation callback the webview fires in parallel will
    // overwrite these fields again with the final URL after redirects;
    // that's fine, the UI gets both updates.
    let (view, was_private) = {
        let mut s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
        let mut private = false;
        if let Some(t) = s.tabs.iter_mut().find(|t| t.id == id) {
            t.url = target.clone();
            t.title = title.clone();
            private = t.private;
        }
        (
            TabsView {
                tabs: s.tabs.clone(),
                active_id: s.active_id,
            },
            private,
        )
    };
    if !was_private {
        record_visit(&app, &target, &title);
    }
    Ok(view)
}

#[tauri::command]
pub fn browser_back(app: tauri::AppHandle, tabs: tauri::State<'_, Tabs>) -> Result<(), String> {
    eval_on_active(&app, &tabs, "history.back()")
}

#[tauri::command]
pub fn browser_forward(app: tauri::AppHandle, tabs: tauri::State<'_, Tabs>) -> Result<(), String> {
    eval_on_active(&app, &tabs, "history.forward()")
}

#[tauri::command]
pub fn browser_reload(app: tauri::AppHandle, tabs: tauri::State<'_, Tabs>) -> Result<(), String> {
    eval_on_active(&app, &tabs, "location.reload()")
}

/// Trigger an in-page text search on the active tab. Uses `window.find`,
/// which highlights the match inside the webview and wraps automatically.
/// `forward = false` searches backward (e.g., Shift+Enter in the find bar).
#[tauri::command]
pub fn browser_find_in_page(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
    query: String,
    forward: bool,
) -> Result<(), String> {
    // window.find(searchString, caseSensitive, backwards, wrapAround,
    //             wholeWord, searchInFrames, showDialog)
    let escaped = query.replace('\\', "\\\\").replace('\'', "\\'");
    let backwards = if forward { "false" } else { "true" };
    let js = format!(
        "window.find && window.find('{escaped}', false, {backwards}, true, false, true, false)"
    );
    eval_on_active(&app, &tabs, &js)
}

/// Clear any in-page find highlighting on the active tab.
#[tauri::command]
pub fn browser_find_clear(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
) -> Result<(), String> {
    eval_on_active(
        &app,
        &tabs,
        "window.getSelection && window.getSelection().removeAllRanges()",
    )
}

#[tauri::command]
pub fn browser_hide_all(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
) -> Result<TabsView, String> {
    let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
    for t in &s.tabs {
        if let Some(wv) = app.get_webview(&tab_label(t.id)) {
            let _ = wv.set_size(LogicalSize::new(0.0, 0.0));
        }
    }
    Ok(TabsView {
        tabs: s.tabs.clone(),
        active_id: s.active_id,
    })
}

/// Re-show the active tab after leaving Dashboard/Settings.
#[tauri::command]
pub fn browser_show_active(
    app: tauri::AppHandle,
    tabs: tauri::State<'_, Tabs>,
) -> Result<TabsView, String> {
    let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
    if let Some(id) = s.active_id {
        if let Some(wv) = app.get_webview(&tab_label(id)) {
            let (pos, size) = active_tab_bounds(&app);
            let _ = wv.set_position(pos);
            let _ = wv.set_size(size);
        }
    }
    Ok(TabsView {
        tabs: s.tabs.clone(),
        active_id: s.active_id,
    })
}

/// Background color for popup webviews. Paints the child webview
/// dark the instant it's created - before the URL loads, before the
/// JS bundle parses, before React mounts - so the user never sees
/// the default white flash. Matches `--bg-elev` (#0d0d0d) in App.css,
/// which the React code sets on `<body>` a few ms later.
const POPUP_BG_COLOR: tauri::webview::Color = tauri::webview::Color(13, 13, 13, 255);

/// Label of the popup webview so open/close are idempotent.
const TRUST_PANEL_LABEL: &str = "trust-panel";
/// Fixed geometry of the site-scan popup. 340 matches the CSS
/// `.trust-panel` width; 460 is the tallest reasonable content box
/// (header + score row + 6-7 signal rows + footnote). If the main
/// window is shorter than that we clamp.
const TRUST_PANEL_WIDTH: f64 = 340.0;
const TRUST_PANEL_MAX_HEIGHT: f64 = 460.0;
const TRUST_PANEL_MARGIN_RIGHT: f64 = 16.0;
const TRUST_PANEL_MARGIN_TOP: f64 = 4.0;

/// Open (or re-point) the site-scan popup as a native child webview
/// of the main window. Because hudsucker/Tauri child webviews stack on
/// the parent in add-order, a panel webview added AFTER the active tab
/// renders ABOVE the tab - which is how we get true overlap without
/// either hiding the page or shrinking its geometry.
///
/// The panel webview loads our own React app with `?panel=trust` so
/// `main.tsx` renders a standalone TrustPopup component (not the full
/// shell). `url` and `tab` are passed as query params the component
/// reads on mount.
#[tauri::command]
pub async fn open_trust_panel(
    app: tauri::AppHandle,
    url: String,
    tab: String,
) -> Result<(), String> {
    let main = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let mobile = mobile_ua_enabled(&app);
    let sidebar_w = if mobile { 0.0 } else { SIDEBAR_WIDTH };
    let chrome_h = if mobile {
        MOBILE_CHROME_HEIGHT
    } else {
        CHROME_HEIGHT
    };
    let (main_w, main_h) = content_size_with(&main, sidebar_w, chrome_h)?;
    let panel_x = (sidebar_w + main_w - TRUST_PANEL_WIDTH - TRUST_PANEL_MARGIN_RIGHT).max(0.0);
    let panel_y = chrome_h + TRUST_PANEL_MARGIN_TOP;
    let panel_h = TRUST_PANEL_MAX_HEIGHT.min((main_h - TRUST_PANEL_MARGIN_TOP - 8.0).max(120.0));

    // Build the URL the popup webview loads. Dev goes through the vite
    // server; release build would swap to tauri://localhost (not wired
    // here because release isn't the current target).
    let base = if cfg!(debug_assertions) {
        "http://localhost:1420"
    } else {
        "http://tauri.localhost"
    };
    let mut popup_url = url::Url::parse(base).map_err(|e| format!("parse base url: {e}"))?;
    popup_url
        .query_pairs_mut()
        .append_pair("panel", "trust")
        .append_pair("url", &url)
        .append_pair("tab", &tab);

    // Close any menu popup so only one popup lives at a time.
    if let Some(wv) = app.get_webview(MENU_POPUP_LABEL) {
        let _ = wv.close();
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let app_clone = app.clone();
    let popup_url_clone = popup_url.clone();
    app.run_on_main_thread(move || {
        // If the panel webview already exists, navigate it to the new
        // URL (which carries new `tab`/`url` params) and reposition;
        // avoids the flash of creating a new webview on every sidebar
        // click.
        if let Some(wv) = app_clone.get_webview(TRUST_PANEL_LABEL) {
            let _ = wv.navigate(popup_url_clone);
            let _ = wv.set_position(LogicalPosition::new(panel_x, panel_y));
            let _ = wv.set_size(LogicalSize::new(TRUST_PANEL_WIDTH, panel_h));
            let _ = tx.send(Ok(()));
            return;
        }
        let res = app_clone
            .get_window("main")
            .ok_or_else(|| "main window not found".to_string())
            .and_then(|main| {
                main.add_child(
                    WebviewBuilder::new(TRUST_PANEL_LABEL, WebviewUrl::External(popup_url_clone))
                        .background_color(POPUP_BG_COLOR),
                    LogicalPosition::new(panel_x, panel_y),
                    LogicalSize::new(TRUST_PANEL_WIDTH, panel_h),
                )
                .map(|_| ())
                .map_err(|e| format!("add_child trust panel: {e}"))
            });
        let _ = tx.send(res);
    })
    .map_err(|e| format!("run_on_main_thread: {e}"))?;

    rx.await
        .map_err(|e| format!("main-thread add_child dropped: {e}"))??;
    Ok(())
}

/// Close the site-scan popup webview if it exists. Idempotent.
#[tauri::command]
pub fn close_trust_panel(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(wv) = app.get_webview(TRUST_PANEL_LABEL) {
        let _ = wv.close();
    }
    Ok(())
}

const MENU_POPUP_LABEL: &str = "menu-popup";
/// Menu popup geometry. Width is comfortable for 1-2 word labels + an
/// icon slot; height scales with item count but a 320 max keeps us
/// inside the smallest mobile window (900 tall) with room to spare.
const MENU_POPUP_WIDTH: f64 = 220.0;
const MENU_POPUP_MAX_HEIGHT: f64 = 320.0;
const MENU_POPUP_MARGIN: f64 = 6.0;

/// Open (or toggle) the hamburger / kebab menu as a child webview. Any
/// existing popup (trust or menu) is closed first so only one popup is
/// ever visible at a time; if the menu is already open with the SAME
/// kind, this closes it (toggle).
///
/// Anchor is the screen-relative top-left of the popup: MobileChrome
/// computes it from the clicked button's bounding rect so the popup
/// visually hangs from the right button.
#[tauri::command]
pub async fn open_menu_popup(
    app: tauri::AppHandle,
    kind: String,
    anchor_x: f64,
    anchor_y: f64,
    view: Option<String>,
    browsing: Option<bool>,
    bookmarked: Option<bool>,
) -> Result<(), String> {
    let main = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let (main_w, main_h) = {
        let size = main.inner_size().map_err(|e| format!("inner_size: {e}"))?;
        let scale = main.scale_factor().map_err(|e| format!("scale: {e}"))?;
        (size.width as f64 / scale, size.height as f64 / scale)
    };

    // Clamp to the window so the popup never renders off-screen.
    let panel_w = MENU_POPUP_WIDTH.min((main_w - 2.0 * MENU_POPUP_MARGIN).max(140.0));
    let panel_h = MENU_POPUP_MAX_HEIGHT.min((main_h - anchor_y - MENU_POPUP_MARGIN).max(120.0));
    let panel_x = anchor_x
        .min(main_w - panel_w - MENU_POPUP_MARGIN)
        .max(MENU_POPUP_MARGIN);
    let panel_y = anchor_y
        .min(main_h - panel_h - MENU_POPUP_MARGIN)
        .max(MENU_POPUP_MARGIN);

    // Close trust panel + any existing menu popup so only one popup
    // lives at a time. If the existing menu was the same kind, treat
    // this as a toggle and return without reopening.
    let existing_same_kind = app
        .get_webview(MENU_POPUP_LABEL)
        .and_then(|wv| wv.url().ok())
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "kind")
                .map(|(_, v)| v.into_owned())
        })
        .map(|k| k == kind)
        .unwrap_or(false);
    if let Some(wv) = app.get_webview(TRUST_PANEL_LABEL) {
        let _ = wv.close();
    }
    if let Some(wv) = app.get_webview(MENU_POPUP_LABEL) {
        let _ = wv.close();
    }
    if let Some(wv) = app.get_webview(crate::context_menu::CONTEXT_MENU_LABEL) {
        let _ = wv.close();
    }
    if existing_same_kind {
        return Ok(());
    }

    let base = if cfg!(debug_assertions) {
        "http://localhost:1420"
    } else {
        "http://tauri.localhost"
    };
    let mut popup_url = url::Url::parse(base).map_err(|e| format!("parse base url: {e}"))?;
    {
        let mut qp = popup_url.query_pairs_mut();
        qp.append_pair("panel", "menu");
        qp.append_pair("kind", &kind);
        if let Some(v) = view.as_deref() {
            qp.append_pair("view", v);
        }
        if let Some(b) = browsing {
            qp.append_pair("browsing", if b { "1" } else { "0" });
        }
        if let Some(b) = bookmarked {
            qp.append_pair("bookmarked", if b { "1" } else { "0" });
        }
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
                    WebviewBuilder::new(MENU_POPUP_LABEL, WebviewUrl::External(popup_url_clone))
                        .background_color(POPUP_BG_COLOR),
                    LogicalPosition::new(panel_x, panel_y),
                    LogicalSize::new(panel_w, panel_h),
                )
                .map(|_| ())
                .map_err(|e| format!("add_child menu popup: {e}"))
            });
        let _ = tx.send(res);
    })
    .map_err(|e| format!("run_on_main_thread: {e}"))?;

    rx.await
        .map_err(|e| format!("main-thread add_child dropped: {e}"))??;
    Ok(())
}

/// Close the menu popup webview if it exists. Idempotent.
#[tauri::command]
pub fn close_menu_popup(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(wv) = app.get_webview(MENU_POPUP_LABEL) {
        let _ = wv.close();
    }
    Ok(())
}

/// Shrink-to-fit the menu popup once React has measured its rendered
/// content. Without this the webview stays at MENU_POPUP_MAX_HEIGHT and
/// leaves dead space below the items (which shows the default white
/// webview background).
#[tauri::command]
pub fn resize_menu_popup(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    if let Some(wv) = app.get_webview(MENU_POPUP_LABEL) {
        let main = app
            .get_window("main")
            .ok_or_else(|| "main window not found".to_string())?;
        let (_, main_h) = {
            let size = main.inner_size().map_err(|e| format!("inner_size: {e}"))?;
            let scale = main.scale_factor().map_err(|e| format!("scale: {e}"))?;
            (size.width as f64 / scale, size.height as f64 / scale)
        };
        let clamped = height.clamp(40.0, main_h - MENU_POPUP_MARGIN * 2.0);
        let _ = wv.set_size(LogicalSize::new(MENU_POPUP_WIDTH, clamped));
    }
    Ok(())
}

/// Close every popup child webview. Used when a React full-screen
/// overlay (tab switcher, CA-trust modal) opens so we don't leave a
/// child webview floating over the now-hidden tab.
#[tauri::command]
pub fn close_all_popups(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(wv) = app.get_webview(TRUST_PANEL_LABEL) {
        let _ = wv.close();
    }
    if let Some(wv) = app.get_webview(MENU_POPUP_LABEL) {
        let _ = wv.close();
    }
    if let Some(wv) = app.get_webview(crate::context_menu::CONTEXT_MENU_LABEL) {
        let _ = wv.close();
    }
    Ok(())
}

/// Re-sync child webview geometry with the current main-window size.
/// Called from the `WindowEvent::Resized` handler installed in
/// `lib.rs`: Tauri's `auto_resize()` would resize ALL children (fights
/// our "inactive tab = 0x0" invariant and reintroduces the second-tab
/// race fixed in #35), so we do it manually and only for the ACTIVE
/// tab + the trust-panel popup.
pub fn resync_layout(app: &tauri::AppHandle) {
    let Some(main) = app.get_window("main") else {
        return;
    };
    let mobile = mobile_ua_enabled(app);
    let sidebar_w = if mobile { 0.0 } else { SIDEBAR_WIDTH };
    let chrome_h = if mobile {
        MOBILE_CHROME_HEIGHT
    } else {
        CHROME_HEIGHT
    };
    let Ok((w, h)) = content_size_with(&main, sidebar_w, chrome_h) else {
        return;
    };

    // Active tab: desktop fills the content area, mobile centers a
    // phone-sized rectangle. Delegated to `active_tab_bounds` so every
    // resize path stays consistent with open/switch/show.
    let active_id = app
        .try_state::<Tabs>()
        .and_then(|state| state.lock().ok().and_then(|g| g.active_id));
    if let Some(id) = active_id {
        if let Some(wv) = app.get_webview(&tab_label(id)) {
            let (pos, size) = active_tab_bounds(app);
            let _ = wv.set_position(pos);
            let _ = wv.set_size(size);
        }
    }

    // Trust-panel popup: stay anchored to top-right. Matches the math
    // in open_trust_panel so a resize after open keeps the popup glued
    // to the URL bar's right-hand side.
    if let Some(wv) = app.get_webview(TRUST_PANEL_LABEL) {
        let panel_x = (sidebar_w + w - TRUST_PANEL_WIDTH - TRUST_PANEL_MARGIN_RIGHT).max(0.0);
        let panel_y = chrome_h + TRUST_PANEL_MARGIN_TOP;
        let panel_h = TRUST_PANEL_MAX_HEIGHT.min((h - TRUST_PANEL_MARGIN_TOP - 8.0).max(120.0));
        let _ = wv.set_position(LogicalPosition::new(panel_x, panel_y));
        let _ = wv.set_size(LogicalSize::new(TRUST_PANEL_WIDTH, panel_h));
    }
}

fn eval_on_active(
    app: &tauri::AppHandle,
    tabs: &tauri::State<'_, Tabs>,
    js: &str,
) -> Result<(), String> {
    let s = tabs.lock().map_err(|e| format!("lock tabs: {e}"))?;
    let id = s.active_id.ok_or_else(|| "no active tab".to_string())?;
    let wv = app
        .get_webview(&tab_label(id))
        .ok_or_else(|| "active tab webview missing".to_string())?;
    wv.eval(js).map_err(|e| format!("eval: {e}"))
}

fn content_size_with<R: tauri::Runtime>(
    main: &tauri::Window<R>,
    sidebar_w: f64,
    chrome_h: f64,
) -> Result<(f64, f64), String> {
    let outer = main.inner_size().map_err(|e| format!("inner_size: {e}"))?;
    let scale = main.scale_factor().map_err(|e| format!("scale: {e}"))?;
    let logical_w = outer.width as f64 / scale;
    let logical_h = outer.height as f64 / scale;
    Ok((
        (logical_w - sidebar_w).max(200.0),
        (logical_h - chrome_h).max(100.0),
    ))
}

/// Short human-readable title from a URL until the webview sends us the real one.
fn host_title(url: &url::Url) -> String {
    if url.scheme() == "data" {
        return "New tab".to_string();
    }
    if url.scheme() == "about" {
        return format!("about:{}", url.path());
    }
    url.host_str()
        .map(|h| h.trim_start_matches("www.").to_string())
        .unwrap_or_else(|| url.to_string())
}

/// Snapshot the current tab list + active-tab index and write it to
/// `<app_data>/session.json`. Fire-and-forget: called after every tab
/// mutation. Data URLs (the new-tab speed dial) are excluded.
fn persist_session(app: &tauri::AppHandle) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        return;
    };
    let Some(state) = app.try_state::<Tabs>() else {
        return;
    };
    let snapshot = {
        let Ok(s) = state.lock() else {
            return;
        };
        let persisted: Vec<_> = s
            .tabs
            .iter()
            .filter(|t| !t.private && !t.url.starts_with("data:") && !t.url.starts_with("about:"))
            .map(|t| crate::session::PersistedTab {
                url: t.url.clone(),
                title: t.title.clone(),
            })
            .collect();
        // active_index refers to `persisted`, which filters out data: tabs.
        let active_index = s
            .active_id
            .and_then(|id| s.tabs.iter().position(|t| t.id == id))
            .and_then(|i| {
                // Map the raw index down through the filter - only tabs
                // that survived make it into `persisted`.
                let mut kept = 0usize;
                for (j, t) in s.tabs.iter().enumerate() {
                    if t.private || t.url.starts_with("data:") || t.url.starts_with("about:") {
                        continue;
                    }
                    if j == i {
                        return Some(kept);
                    }
                    kept += 1;
                }
                None
            });
        crate::session::Session {
            tabs: persisted,
            active_index,
        }
    };
    if let Err(e) = crate::session::save(&data_dir, &snapshot) {
        tracing::debug!(error = %e, "session save failed");
    }
}

/// Kick off a background favicon fetch for a host if we haven't already
/// tried. Results are cached on disk under `<app_data>/favicons/` so the
/// next run + every subsequent tab to the same host stays cheap.
async fn maybe_fetch_favicon(app: &tauri::AppHandle, host: String) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        return;
    };
    if crate::favicons::is_cached(&data_dir, &host) {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::favicons::fetch_and_cache(&data_dir, &host).await {
            tracing::debug!(host = %host, error = %e, "favicon fetch failed");
        }
        // Frontend polls via get_favicon, but a soft nudge helps it refresh
        // promptly without waiting for a tab-strip rerender.
        let _ = handle.emit("blueflame:favicon-ready", host);
    });
}

/// Turn whatever the user typed into the URL bar into an `http(s)` URL.
/// Plain words or multi-word queries route to the user's chosen search engine.
fn resolve_url(input: &str, engine: SearchEngine) -> anyhow::Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty input");
    }

    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("about:")
    {
        return Ok(trimmed.to_string());
    }

    if !trimmed.contains(' ') && trimmed.contains('.') {
        return Ok(format!("https://{trimmed}"));
    }

    Ok(engine.search_url(trimmed))
}

/// Load the user's chosen engine, falling back to the default on any error so
/// the URL bar never goes unresponsive because of a bad settings file.
fn current_engine(app: &tauri::AppHandle) -> SearchEngine {
    let Ok(data) = app.path().app_data_dir() else {
        return SearchEngine::default();
    };
    match SearchSettings::open(&data) {
        Ok(settings) => settings.get_engine().unwrap_or_default(),
        Err(_) => SearchEngine::default(),
    }
}

/// Read the persisted "mobile UA" toggle. Defaults to `false` (desktop
/// UA) on any error - we'd rather silently stay in desktop mode than
/// flip the whole browsing session on a transient read failure.
fn mobile_ua_enabled(app: &tauri::AppHandle) -> bool {
    let Ok(data) = app.path().app_data_dir() else {
        return false;
    };
    match SearchSettings::open(&data) {
        Ok(settings) => settings.get_mobile_ua().unwrap_or(false),
        Err(_) => false,
    }
}

/// Main-window dimensions applied when the user flips the
/// desktop/mobile toggle. Mobile picks a phone-ish portrait shape
/// (just wide enough for the 412 viewport + sidebar + a few px of
/// breathing room); desktop restores the tauri.conf.json defaults.
const MOBILE_WINDOW_WIDTH: f64 = 480.0;
const MOBILE_WINDOW_HEIGHT: f64 = 900.0;
const DESKTOP_WINDOW_WIDTH: f64 = 1200.0;
const DESKTOP_WINDOW_HEIGHT: f64 = 760.0;

/// Resize the main window to match the selected browser mode AND
/// flip `resizable` so the shape is locked while mobile, free while
/// desktop. Called from `set_mobile_ua` after the tab rebuild.
///
/// Mobile path: snapshot the current window size (if it isn't
/// already the mobile lock - avoids overwriting a good snapshot with
/// a stale mobile lock), then force 480 x 900 and
/// `set_resizable(false)`.
///
/// Desktop path: read the saved snapshot (or fall back to defaults),
/// `set_resizable(true)` first so the set_size isn't clamped by a
/// stale min/max, then restore.
pub fn apply_window_size_for_mode(app: &tauri::AppHandle, mobile: bool) {
    let Some(main) = app.get_window("main") else {
        return;
    };

    let settings = app
        .path()
        .app_data_dir()
        .ok()
        .and_then(|d| crate::search::SearchSettings::open(&d).ok());

    if mobile {
        // Remember what the user was using on desktop so flipping
        // back takes them there. Skip the capture if the current
        // window looks like we're already in mobile mode (e.g. on
        // boot re-apply) - a 480-ish width would overwrite the real
        // desktop size with a stale mobile value.
        if let (Ok(size), Ok(scale)) = (main.inner_size(), main.scale_factor()) {
            let logical_w = size.width as f64 / scale;
            let logical_h = size.height as f64 / scale;
            let looks_like_mobile =
                (logical_w - MOBILE_WINDOW_WIDTH).abs() < 40.0 && logical_h <= 960.0;
            if !looks_like_mobile {
                if let Some(ref s) = settings {
                    if let Err(e) = s.set_desktop_window_size(logical_w, logical_h) {
                        tracing::warn!(error = %e, "failed to snapshot desktop window size");
                    }
                }
            }
        }
        if let Err(e) = main.set_size(LogicalSize::new(MOBILE_WINDOW_WIDTH, MOBILE_WINDOW_HEIGHT)) {
            tracing::warn!(error = %e, "failed to resize main window to mobile");
        }
        if let Err(e) = main.set_resizable(false) {
            tracing::warn!(error = %e, "failed to lock main window in mobile mode");
        }
    } else {
        // Restore desktop. Flip resizable FIRST so the subsequent
        // set_size isn't constrained by the mobile-mode min/max.
        if let Err(e) = main.set_resizable(true) {
            tracing::warn!(error = %e, "failed to unlock main window on desktop flip");
        }
        let (w, h) = settings
            .as_ref()
            .and_then(|s| s.get_desktop_window_size().ok().flatten())
            .unwrap_or((DESKTOP_WINDOW_WIDTH, DESKTOP_WINDOW_HEIGHT));
        if let Err(e) = main.set_size(LogicalSize::new(w, h)) {
            tracing::warn!(error = %e, "failed to restore desktop window size");
        }
    }
}

/// Position + size the active tab's webview should occupy, given the
/// current window size and the desktop/mobile toggle. Desktop mode
/// fills the whole content area below the chrome; mobile mode
/// centers a narrow phone-sized rectangle so the user sees actual
/// mobile-browser emulation (and sites see a genuinely mobile
/// viewport width, which is what makes UA-sniffing detection stick).
///
/// Used from every code path that places the active tab: add_child on
/// open, switch_tab, show_active, close_tab's fallback, and
/// resync_layout on window resize. Centralizing keeps mobile-mode
/// emulation consistent across all of them.
pub(crate) fn active_tab_bounds(
    app: &tauri::AppHandle,
) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    let Some(main) = app.get_window("main") else {
        return (
            LogicalPosition::new(SIDEBAR_WIDTH, CHROME_HEIGHT),
            LogicalSize::new(800.0, 600.0),
        );
    };
    let mobile = mobile_ua_enabled(app);
    // Sidebar is hidden in mobile mode (see `.mobile-shell .sidebar`
    // CSS), so its 48px should not reserve space on the left.
    let sidebar_w = if mobile { 0.0 } else { SIDEBAR_WIDTH };
    let chrome_h = if mobile {
        MOBILE_CHROME_HEIGHT
    } else {
        CHROME_HEIGHT
    };
    let (full_w, full_h) = match (main.inner_size(), main.scale_factor()) {
        (Ok(size), Ok(scale)) => (size.width as f64 / scale, size.height as f64 / scale),
        _ => (800.0, 600.0),
    };
    let content_w = (full_w - sidebar_w).max(200.0);
    let content_h = (full_h - chrome_h).max(100.0);
    // Mobile mode: webview fills the whole content area (the 480px locked
    // window IS the phone). Earlier we centered a 412px rectangle inside
    // a wider desktop window for phone emulation, but with the window
    // locked to mobile dimensions that just leaves dead gutters on each
    // side where the chrome visually detaches from the page. UA + screen
    // spoofing in the init script covers the phone-detection signals.
    (
        LogicalPosition::new(sidebar_w, chrome_h),
        LogicalSize::new(content_w, content_h),
    )
}

fn record_visit(app: &tauri::AppHandle, url: &str, title: &str) {
    if let Some(store) = app.try_state::<crate::storage::SharedStore>() {
        if let Ok(guard) = store.lock() {
            let _ = guard.record_visit(url, title);
        }
    }
}

fn metasearch_enabled(app: &tauri::AppHandle) -> bool {
    let Ok(data) = app.path().app_data_dir() else {
        return false;
    };
    match SearchSettings::open(&data) {
        Ok(settings) => settings.get_metasearch().unwrap_or(false),
        Err(_) => false,
    }
}

/// Match the same heuristic as `resolve_url`: anything that isn't a URL
/// (scheme or bare host) counts as a search query.
fn looks_like_search_query(input: &str) -> bool {
    let t = input.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("data:")
        || t.starts_with("about:")
    {
        return false;
    }
    // Bare host with a dot and no spaces is a URL, not a query
    !t.contains('.') || t.contains(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_url_passes_through() {
        assert_eq!(
            resolve_url("https://example.com", SearchEngine::DuckDuckGo).unwrap(),
            "https://example.com"
        );
    }

    #[test]
    fn bare_host_gets_https() {
        assert_eq!(
            resolve_url("github.com", SearchEngine::DuckDuckGo).unwrap(),
            "https://github.com"
        );
    }

    #[test]
    fn search_routes_to_chosen_engine() {
        let ddg = resolve_url("best browser", SearchEngine::DuckDuckGo).unwrap();
        assert!(ddg.starts_with("https://duckduckgo.com/"));
        let brave = resolve_url("best browser", SearchEngine::Brave).unwrap();
        assert!(brave.starts_with("https://search.brave.com/"));
    }

    #[test]
    fn empty_errors() {
        assert!(resolve_url("", SearchEngine::DuckDuckGo).is_err());
        assert!(resolve_url("   ", SearchEngine::DuckDuckGo).is_err());
    }

    #[test]
    fn tab_label_is_stable_and_unique() {
        assert_eq!(tab_label(1), "browse-1");
        assert_ne!(tab_label(1), tab_label(2));
    }

    #[test]
    fn host_title_strips_www() {
        let u: url::Url = "https://www.github.com/w1ck3ds0d4".parse().unwrap();
        assert_eq!(host_title(&u), "github.com");
    }

    #[test]
    fn issue_id_is_monotonic() {
        let mut s = TabsState::default();
        assert_eq!(s.issue_id(), 1);
        assert_eq!(s.issue_id(), 2);
        assert_eq!(s.issue_id(), 3);
    }
}
