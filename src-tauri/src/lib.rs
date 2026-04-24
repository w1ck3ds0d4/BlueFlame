mod body_analysis;
mod browser;
mod ca;
mod ca_trust;
mod commands;
mod context_menu;
mod debug_log;
#[cfg(feature = "built-in-tor")]
mod embedded_tor;
mod favicons;
mod filter_parser;
mod import_export;
mod list_loader;
mod metasearch;
mod metrics;
mod new_tab;
mod proxy;
mod reputation;
mod search;
mod security;
mod session;
mod socks_connector;
mod storage;
mod tls_verifier;
mod trust;
mod util;

use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use browser::{
    browser_back, browser_close_tab, browser_find_clear, browser_find_in_page, browser_forward,
    browser_hide_all, browser_list_tabs, browser_navigate_active, browser_new_private_tab,
    browser_new_tab, browser_open_tab, browser_reload, browser_show_active, browser_switch_tab,
    close_all_popups, close_menu_popup, close_trust_panel, open_menu_popup, open_trust_panel,
    resize_menu_popup, Tabs,
};
use commands::{
    bookmark_delete_folder, bookmark_folders, bookmark_is, bookmark_list, bookmark_rename_folder,
    bookmark_set_folder, bookmark_toggle, clear_block_log, clear_debug_log, disable_filters,
    enable_filters, get_blocks_for_host, get_ca_cert_path, get_ca_trust_status, get_debug_log,
    get_favicon, get_filter_lists, get_metasearch_enabled, get_mobile_ua, get_proxy_status,
    get_recent_blocks, get_reputation_feeds, get_search_engine, get_stats, get_system_summary,
    get_tor_settings, get_trust, get_trust_history, install_ca, list_search_engines,
    log_from_frontend, personal_clear_history, personal_recent, personal_search,
    personal_top_visited, refresh_filter_lists, refresh_reputation_feeds, reset_stats, reveal_ca,
    set_metasearch_enabled, set_mobile_ua, set_search_engine, set_tor_settings, url_suggest,
};
use context_menu::{
    close_context_menu, get_context_payload, resize_context_menu, ContextStore,
    SharedContextMenuTx, SharedContextStore, SharedContextToken,
};
use import_export::{export_data, import_bookmarks_html, import_data};
use metrics::{get_system_metrics, MetricsCollector, SharedMetrics};
use proxy::ProxyState;
use storage::{SharedStore, Store};

/// Default port for the local MITM proxy. Kept stable so users can also
/// point external browsers at it if they want to experiment.
pub(crate) const PROXY_PORT: u16 = 18080;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install rustls' ring crypto provider as the process default
    // BEFORE any rustls code runs. Our dep tree enables both `ring`
    // (via arti) and `aws-lc-rs` (via rustls default features), and
    // rustls 0.23 panics in `ClientConfig::builder()` when it can't
    // pick one unambiguously from crate features. Explicit install
    // resolves that: the MITM proxy's TLS config, the reputation feed
    // fetcher, and every other rustls-using crate in this process
    // now share a known-good provider. install_default() returns Err
    // if someone else already installed one (harmless race loss).
    let _ = rustls::crypto::ring::default_provider().install_default();

    use tracing_subscriber::prelude::*;

    let debug_log: debug_log::SharedDebugLog = Arc::new(debug_log::DebugLog::default());

    // hudsucker::proxy::internal emits ERROR for every TLS connection
    // that closes without a close_notify alert (documented rustls
    // `unexpected_eof` noise: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof)
    // and for generic transient connect errors whose message ("HTTPS
    // connect error: connection error") carries no actionable detail.
    // Both are expected during normal browsing through a MITM proxy
    // and spam the log + the in-app Debug monitor. Silence the module
    // in the default filter. Users debugging a genuine proxy bug can
    // opt back in via `RUST_LOG=hudsucker=debug`.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "blueflame=info,hudsucker=warn,hudsucker::proxy::internal=off".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(debug_log::DebugLogLayer::new(debug_log.clone()))
        .init();

    // Tell the system WebView to route all traffic through our proxy *before*
    // Tauri creates any windows. This must happen before the webview is spawned.
    configure_webview_proxy(PROXY_PORT);

    let proxy_state: Arc<Mutex<ProxyState>> = Arc::new(Mutex::new(ProxyState::default()));

    // Context-menu plumbing: per-launch token + ipc channel + payload
    // store. The channel sender is exposed to the proxy so the
    // sentinel handler can push requests; the receiver is owned by a
    // consumer task spawned below that has an AppHandle and opens
    // the popup. The store holds one payload per pending popup keyed
    // by UUID - the popup looks itself up via `get_context_payload`.
    let context_token: SharedContextToken = context_menu::fresh_token();
    let context_store: SharedContextStore = std::sync::Arc::new(ContextStore::default());
    let (context_tx, context_rx) =
        tokio::sync::mpsc::unbounded_channel::<context_menu::ContextMenuRequest>();
    let context_tx_shared: SharedContextMenuTx =
        std::sync::Arc::new(std::sync::Mutex::new(Some(context_tx)));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(proxy_state.clone())
        .manage(Tabs::default())
        .manage(debug_log.clone())
        .manage(context_token.clone())
        .manage(context_store.clone())
        .manage(context_tx_shared.clone())
        .manage::<SharedMetrics>(Arc::new(MetricsCollector::default()))
        .setup(move |app| {
            // Open the personal-index store so commands can rely on it being in state.
            let data = app.path().app_data_dir()?;
            let store = Store::open(&data)?;
            app.manage::<SharedStore>(std::sync::Mutex::new(store));

            let app_handle = app.handle().clone();
            let proxy_state = proxy_state.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = start_proxy_at_boot(&app_handle, proxy_state, PROXY_PORT).await {
                    tracing::error!(error = ?e, "failed to auto-start proxy at boot");
                }
            });

            // Context-menu consumer: drain the mpsc receiver the proxy
            // sentinel handler feeds into. `Open` requests spawn a
            // popup (translating tab-webview-local click coordinates
            // to main-window coordinates via the active tab bounds);
            // `Dismiss` requests close any open popup.
            let mut rx: context_menu::ContextMenuRx = context_rx;
            let consumer_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(req) = rx.recv().await {
                    match req {
                        context_menu::ContextMenuRequest::Open(mut payload) => {
                            let (offset, _) = browser::active_tab_bounds(&consumer_handle);
                            payload.screen_x += offset.x;
                            payload.screen_y += offset.y;
                            if let Err(e) =
                                context_menu::open_context_menu_popup(&consumer_handle, payload)
                                    .await
                            {
                                tracing::warn!(error = %e, "open context menu failed");
                            }
                        }
                        context_menu::ContextMenuRequest::Dismiss => {
                            if let Some(wv) =
                                consumer_handle.get_webview(context_menu::CONTEXT_MENU_LABEL)
                            {
                                let _ = wv.close();
                            }
                        }
                    }
                }
            });

            // If the user previously flipped into mobile mode and
            // restarted, resize the OS window to the phone-ish shape
            // before the user sees it. Reads the persisted setting;
            // missing or unreadable falls through to desktop defaults.
            if let Ok(data) = app.path().app_data_dir() {
                if let Ok(settings) = search::SearchSettings::open(&data) {
                    if settings.get_mobile_ua().unwrap_or(false) {
                        browser::apply_window_size_for_mode(app.handle(), true);
                    }
                }
            }

            // Keep the active tab + trust popup in sync with the main
            // window's current size. Tauri's auto_resize() would do
            // this for us but applies to EVERY child (including hidden
            // tabs we keep at 0x0), so we do it manually for just the
            // visible surfaces.
            if let Some(main_win) = app.get_window("main") {
                let resize_handle = app.handle().clone();
                main_win.on_window_event(move |event| {
                    if matches!(
                        event,
                        tauri::WindowEvent::Resized(_)
                            | tauri::WindowEvent::ScaleFactorChanged { .. }
                    ) {
                        browser::resync_layout(&resize_handle);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            enable_filters,
            disable_filters,
            get_proxy_status,
            get_stats,
            get_ca_cert_path,
            get_ca_trust_status,
            install_ca,
            reveal_ca,
            refresh_filter_lists,
            get_filter_lists,
            reset_stats,
            get_recent_blocks,
            clear_block_log,
            browser_open_tab,
            browser_new_tab,
            browser_switch_tab,
            browser_close_tab,
            browser_list_tabs,
            browser_navigate_active,
            browser_back,
            browser_forward,
            browser_reload,
            browser_hide_all,
            browser_show_active,
            get_search_engine,
            set_search_engine,
            list_search_engines,
            get_metasearch_enabled,
            set_metasearch_enabled,
            get_mobile_ua,
            set_mobile_ua,
            personal_search,
            personal_recent,
            personal_clear_history,
            personal_top_visited,
            url_suggest,
            bookmark_toggle,
            bookmark_is,
            bookmark_list,
            bookmark_set_folder,
            bookmark_folders,
            bookmark_rename_folder,
            bookmark_delete_folder,
            get_system_summary,
            get_debug_log,
            clear_debug_log,
            log_from_frontend,
            get_tor_settings,
            set_tor_settings,
            get_favicon,
            browser_find_in_page,
            browser_find_clear,
            get_blocks_for_host,
            browser_new_private_tab,
            open_trust_panel,
            close_trust_panel,
            open_menu_popup,
            close_menu_popup,
            resize_menu_popup,
            close_all_popups,
            get_trust,
            get_trust_history,
            get_reputation_feeds,
            refresh_reputation_feeds,
            close_context_menu,
            resize_context_menu,
            get_context_payload,
            export_data,
            import_data,
            import_bookmarks_html,
            get_system_metrics,
        ])
        .run(tauri::generate_context!())
        .expect("error while running BlueFlame");
}

/// Configure the system WebView to route through our proxy.
///
/// - Windows: WebView2 reads `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` at startup.
/// - Linux: WebKit2GTK reads standard `http_proxy` / `https_proxy` env vars.
/// - macOS: WKWebView ignores these; proxy config must be set at the
///   `WKWebsiteDataStore` level from native code (deferred to a later PR).
fn configure_webview_proxy(port: u16) {
    let proxy_url = format!("http://127.0.0.1:{port}");

    #[cfg(target_os = "windows")]
    {
        let args = format!("--proxy-server={proxy_url}");
        // Appends to any existing value so users can still pass custom flags via their env.
        let existing = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
        let combined = if existing.is_empty() {
            args
        } else {
            format!("{existing} {args}")
        };
        // SAFETY: set_var is unsafe in newer Rust; we call it before any webview spawns.
        unsafe {
            std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", combined);
        }
    }

    #[cfg(target_os = "linux")]
    {
        // SAFETY: set before webview init.
        unsafe {
            std::env::set_var("http_proxy", &proxy_url);
            std::env::set_var("https_proxy", &proxy_url);
        }
    }

    #[cfg(target_os = "macos")]
    {
        // macOS WKWebView ignores env-var proxies entirely. Instead,
        // each WebviewBuilder in `browser.rs` (tabs + trust popup)
        // passes `.proxy_url(http://127.0.0.1:{port})` at creation
        // time - that hits Tauri's `macos-proxy` feature which wraps
        // wry's `mac-proxy` (macOS 14+ Network-framework API).
        let _ = proxy_url;
    }
}

async fn start_proxy_at_boot(
    app: &tauri::AppHandle,
    proxy_state: Arc<Mutex<ProxyState>>,
    port: u16,
) -> anyhow::Result<()> {
    let data_dir = app.path().app_data_dir()?;
    let ca_dir = data_dir.join("ca");
    let root_ca = ca::load_or_create(&ca_dir)?;

    let (filters, filters_enabled, stats, block_log, security) = {
        let s = proxy_state.lock().await;
        (
            s.filters.clone(),
            s.filters_enabled.clone(),
            s.stats.clone(),
            s.block_log.clone(),
            s.security.clone(),
        )
    };

    let (upstream, upstream_applied) = resolve_upstream(&data_dir, &proxy_state).await;

    // Copy the token/channel Arcs out of Tauri state so the proxy
    // handler can match the sentinel host requests without holding a
    // State<'_> across await points. Cloning an Arc is just a ref-
    // count bump; the actual String isn't duplicated.
    let context_token: std::sync::Arc<String> = (*app.state::<SharedContextToken>()).clone();
    let context_tx = (*app.state::<SharedContextMenuTx>()).clone();

    let runner = proxy::start(
        port,
        root_ca,
        filters,
        filters_enabled,
        stats,
        block_log,
        security,
        upstream,
        context_token,
        context_tx,
    )
    .await?;

    {
        let mut s = proxy_state.lock().await;
        s.running = true;
        s.port = port;
        s.runner = Some(runner);
        s.started_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs());
        s.upstream_applied = upstream_applied;
    }

    tracing::info!(port, "proxy ready");

    // Restore last session's tabs so the user picks up where they left off.
    // Fire-and-forget in the background - tab opens are async (run on the
    // main thread) and we don't want to block proxy boot on them.
    let restore_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        restore_session(&restore_handle).await;
    });

    // Kick off filter-list loading in the background. The proxy runs against
    // the built-in defaults until the loader swaps in a richer set.
    let cache_dir = data_dir.join("filter-cache");
    let state_for_loader = proxy_state.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = hydrate_filter_lists(state_for_loader, cache_dir).await {
            tracing::warn!(error = ?e, "filter-list hydration failed, using built-in defaults");
        }
    });

    // Reputation feeds hydrate on the same two-pass pattern: cached hosts
    // first so the scorer is useful immediately, then the network fetch
    // extends the store when it completes.
    let rep_cache_dir = reputation::cache_dir(&data_dir);
    let state_for_rep = proxy_state.clone();
    tauri::async_runtime::spawn(async move {
        hydrate_reputation_feeds(state_for_rep, rep_cache_dir).await;
    });

    Ok(())
}

/// Mirror of `hydrate_filter_lists` for reputation feeds: load any
/// cached copies into the in-memory host set, then attempt a fresh
/// fetch per feed in the background. Fetch failures are non-fatal -
/// the bundled baseline plus whatever cached list exists still works.
async fn hydrate_reputation_feeds(
    proxy_state: Arc<Mutex<ProxyState>>,
    cache_dir: std::path::PathBuf,
) {
    std::fs::create_dir_all(&cache_dir).ok();
    let feeds = reputation::default_feeds();

    let store = {
        let s = proxy_state.lock().await;
        s.reputation.clone()
    };

    let mut cached_added = 0usize;
    for feed in &feeds {
        if let Some(hosts) = reputation::load_cached(feed, &cache_dir) {
            cached_added += hosts.len();
            store.extend(hosts);
        }
    }
    if cached_added > 0 {
        tracing::info!(
            hosts = cached_added,
            "loaded cached reputation feeds into store"
        );
    }

    let mut fresh_added = 0usize;
    for feed in &feeds {
        match reputation::fetch_and_cache(feed, &cache_dir).await {
            Ok(hosts) => {
                fresh_added += hosts.len();
                store.extend(hosts);
            }
            Err(e) => {
                tracing::warn!(error = ?e, name = %feed.name, "reputation feed fetch failed");
            }
        }
    }
    if fresh_added > 0 {
        tracing::info!(
            hosts = fresh_added,
            "refreshed reputation feeds from network"
        );
    }
}

/// Load cached filter lists into the active RegexSet, then refresh from the
/// network in the background. Any list that fails to fetch is ignored so the
/// user gets partial protection rather than none.
async fn hydrate_filter_lists(
    proxy_state: Arc<Mutex<ProxyState>>,
    cache_dir: std::path::PathBuf,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&cache_dir).ok();
    let lists = list_loader::default_lists();

    // Pass 1: load whatever is cached, compile a merged set, swap in.
    let mut bodies: Vec<String> = Vec::new();
    for list in &lists {
        let path = list_loader::cache_path(&cache_dir, &list.url);
        if let Ok((body, _)) = list_loader::read_cache(&path) {
            bodies.push(body);
        }
    }
    if !bodies.is_empty() {
        let set = list_loader::compile_patterns(&proxy::default_filter_patterns(), &bodies)?;
        let state = proxy_state.lock().await;
        proxy::swap_filters(&state, set);
        tracing::info!(lists = bodies.len(), "loaded cached filter lists");
    }

    // Pass 2: fetch fresh copies in the background, persist to cache, recompile.
    let mut fresh: Vec<String> = Vec::new();
    for list in &lists {
        match list_loader::fetch(&list.url).await {
            Ok(body) => {
                let path = list_loader::cache_path(&cache_dir, &list.url);
                if let Err(e) = list_loader::write_cache(&path, &body) {
                    tracing::warn!(error = ?e, name = %list.name, "failed to cache list");
                }
                fresh.push(body);
            }
            Err(e) => {
                tracing::warn!(error = ?e, name = %list.name, "failed to fetch list");
            }
        }
    }
    if !fresh.is_empty() {
        let set = list_loader::compile_patterns(&proxy::default_filter_patterns(), &fresh)?;
        let state = proxy_state.lock().await;
        proxy::swap_filters(&state, set);
        tracing::info!(lists = fresh.len(), "refreshed filter lists from network");
    }

    Ok(())
}

/// Read `<app_data>/session.json` and re-open every tab the user had on
/// last exit. The frontend auto-opens a fresh new-tab on boot when it
/// sees zero tabs, so leaving the session file empty is a safe default.
async fn restore_session(app: &tauri::AppHandle) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        return;
    };
    let Some(sess) = session::load(&data_dir) else {
        return;
    };
    if sess.tabs.is_empty() {
        return;
    }
    tracing::info!(count = sess.tabs.len(), "restoring tab session");

    let target_active = sess.active_index;
    let mut last_id: Option<u64> = None;
    for tab in &sess.tabs {
        let tabs_state = app.state::<browser::Tabs>();
        match browser::browser_open_tab(app.clone(), tabs_state, tab.url.clone()).await {
            Ok(view) => {
                last_id = view.active_id;
            }
            Err(e) => {
                tracing::warn!(url = %tab.url, error = %e, "failed to restore tab");
            }
        }
    }
    if let (Some(idx), Some(_)) = (target_active, last_id) {
        let tabs_state = app.state::<browser::Tabs>();
        let switch_id = tabs_state.lock().ok().and_then(|s| s.id_at(idx));
        if let Some(id) = switch_id {
            let _ = browser::browser_switch_tab(app.clone(), app.state::<browser::Tabs>(), id);
        }
    }
}

/// Decide at boot which upstream the MITM proxy should use, based on the
/// persisted settings. Returns the `Upstream` to feed into `proxy::start`
/// and a short display label for `ProxyState.upstream_applied`.
/// Changing the settings at runtime requires an app restart to take effect.
///
/// While the embedded Tor client is bootstrapping, `ProxyState.tor_bootstrap`
/// is updated so the UI can render a loading indicator:
/// `""` -> `"running"` -> (`"ready"` | `"failed:<message>"`).
async fn resolve_upstream(
    data_dir: &std::path::Path,
    proxy_state: &Arc<Mutex<ProxyState>>,
) -> (proxy::Upstream, String) {
    let Ok(settings) = search::SearchSettings::open(data_dir) else {
        return (proxy::Upstream::Direct, "direct".to_string());
    };

    #[cfg(feature = "built-in-tor")]
    if settings.get_tor_builtin().unwrap_or(false) {
        {
            let mut s = proxy_state.lock().await;
            s.tor_bootstrap = "running".to_string();
        }
        match embedded_tor::bootstrap(data_dir).await {
            Ok(tor) => {
                {
                    let mut s = proxy_state.lock().await;
                    s.tor_bootstrap = "ready".to_string();
                }
                return (proxy::Upstream::BuiltInTor(tor), "built-in-tor".to_string());
            }
            Err(e) => {
                {
                    let mut s = proxy_state.lock().await;
                    s.tor_bootstrap = format!("failed:{e}");
                }
                tracing::error!(error = ?e, "embedded Tor bootstrap failed - falling back to direct");
            }
        }
    }

    if settings.get_tor_enabled().unwrap_or(false) {
        if let Ok(addr_s) = settings.get_tor_proxy_addr() {
            match addr_s.parse::<std::net::SocketAddr>() {
                Ok(a) => return (proxy::Upstream::Socks5(a), format!("socks5:{a}")),
                Err(e) => {
                    tracing::warn!(addr = %addr_s, error = %e, "tor_proxy_addr failed to parse");
                }
            }
        }
    }

    (proxy::Upstream::Direct, "direct".to_string())
}
