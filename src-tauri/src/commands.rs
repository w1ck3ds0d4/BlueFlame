//! Tauri commands invoked from the frontend.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

use crate::proxy::{ProxyState, StatsSnapshot};

type State<'r> = tauri::State<'r, Arc<Mutex<ProxyState>>>;

/// UI-facing status for the proxy.
#[derive(serde::Serialize)]
pub struct ProxyStatus {
    /// Whether the proxy listener is up (nearly always true; proxy auto-starts at boot).
    pub running: bool,
    /// Port the proxy is bound to.
    pub port: u16,
    /// Whether filter matching is active.
    pub filters_enabled: bool,
    /// Embedded-Tor bootstrap state: `""`, `"running"`, `"ready"`, or
    /// `"failed:<message>"`. Surfaced so the chrome can show a spinner while
    /// arti fetches the initial directory consensus (~10-30s on first run).
    pub tor_bootstrap: String,
}

#[tauri::command]
pub async fn get_proxy_status(state: State<'_>) -> Result<ProxyStatus, String> {
    let s = state.lock().await;
    Ok(ProxyStatus {
        running: s.running,
        port: s.port,
        filters_enabled: s.filters_enabled.load(Ordering::Relaxed),
        tor_bootstrap: s.tor_bootstrap.clone(),
    })
}

#[tauri::command]
pub async fn enable_filters(state: State<'_>) -> Result<(), String> {
    let s = state.lock().await;
    s.filters_enabled.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn disable_filters(state: State<'_>) -> Result<(), String> {
    let s = state.lock().await;
    s.filters_enabled.store(false, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn get_stats(state: State<'_>) -> Result<StatsSnapshot, String> {
    let s = state.lock().await;
    Ok(s.stats.snapshot())
}

#[tauri::command]
pub async fn get_search_engine(app: tauri::AppHandle) -> Result<String, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings =
        crate::search::SearchSettings::open(&data).map_err(|e| format!("open settings: {e}"))?;
    Ok(settings
        .get_engine()
        .map_err(|e| format!("get: {e}"))?
        .id()
        .to_string())
}

#[tauri::command]
pub async fn set_search_engine(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let engine =
        crate::search::SearchEngine::from_id(&id).ok_or_else(|| format!("unknown engine {id}"))?;
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings =
        crate::search::SearchSettings::open(&data).map_err(|e| format!("open settings: {e}"))?;
    settings.set_engine(engine).map_err(|e| format!("set: {e}"))
}

#[derive(serde::Serialize)]
pub struct EngineOption {
    pub id: String,
    pub name: String,
}

#[tauri::command]
pub async fn list_search_engines() -> Vec<EngineOption> {
    crate::search::SearchEngine::all()
        .iter()
        .map(|e| EngineOption {
            id: e.id().to_string(),
            name: e.display_name().to_string(),
        })
        .collect()
}

#[tauri::command]
pub async fn get_metasearch_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings =
        crate::search::SearchSettings::open(&data).map_err(|e| format!("open settings: {e}"))?;
    settings.get_metasearch().map_err(|e| format!("get: {e}"))
}

#[tauri::command]
pub async fn personal_search(
    store: tauri::State<'_, crate::storage::SharedStore>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<crate::storage::Visit>, String> {
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    let take = limit.unwrap_or(30).min(200);
    s.search_history(&query, take)
        .map_err(|e| format!("search: {e}"))
}

#[tauri::command]
pub async fn personal_recent(
    store: tauri::State<'_, crate::storage::SharedStore>,
    limit: Option<usize>,
) -> Result<Vec<crate::storage::Visit>, String> {
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    let take = limit.unwrap_or(30).min(200);
    s.recent_history(take).map_err(|e| format!("recent: {e}"))
}

#[tauri::command]
pub async fn personal_clear_history(
    store: tauri::State<'_, crate::storage::SharedStore>,
) -> Result<(), String> {
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    s.clear_history().map_err(|e| format!("clear: {e}"))
}

#[tauri::command]
pub async fn personal_top_visited(
    store: tauri::State<'_, crate::storage::SharedStore>,
    limit: Option<usize>,
) -> Result<Vec<crate::storage::Visit>, String> {
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    let take = limit.unwrap_or(12).min(100);
    s.top_visited(take).map_err(|e| format!("top: {e}"))
}

#[tauri::command]
pub async fn url_suggest(
    store: tauri::State<'_, crate::storage::SharedStore>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<crate::storage::Suggestion>, String> {
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    let take = limit.unwrap_or(8).min(20);
    s.suggest(&query, take).map_err(|e| format!("suggest: {e}"))
}

#[tauri::command]
pub fn get_debug_log(
    log: tauri::State<'_, crate::debug_log::SharedDebugLog>,
    limit: Option<usize>,
) -> Vec<crate::debug_log::DebugEntry> {
    let take = limit.unwrap_or(200).min(500);
    log.recent(take)
}

#[tauri::command]
pub fn clear_debug_log(log: tauri::State<'_, crate::debug_log::SharedDebugLog>) {
    log.clear();
}

/// Pipe a frontend-side event (e.g. an uncaught JS error) into the same
/// ring buffer the Rust `tracing` events land in, so the Debug view shows
/// one merged feed.
#[tauri::command]
pub fn get_favicon(app: tauri::AppHandle, host: String) -> Option<String> {
    let data = app.path().app_data_dir().ok()?;
    let (mime, body) = crate::favicons::read(&data, &host)?;
    if mime == "x-miss" || body.is_empty() {
        return None;
    }
    Some(crate::favicons::data_url(&mime, &body))
}

#[tauri::command]
pub fn log_from_frontend(
    log: tauri::State<'_, crate::debug_log::SharedDebugLog>,
    level: String,
    message: String,
    target: Option<String>,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    log.push(crate::debug_log::DebugEntry {
        ts: now,
        level,
        target: target.unwrap_or_else(|| "frontend".to_string()),
        message,
    });
}

#[tauri::command]
pub async fn bookmark_toggle(
    store: tauri::State<'_, crate::storage::SharedStore>,
    url: String,
    title: String,
) -> Result<bool, String> {
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    s.toggle_bookmark(&url, &title)
        .map_err(|e| format!("toggle: {e}"))
}

#[tauri::command]
pub async fn bookmark_is(
    store: tauri::State<'_, crate::storage::SharedStore>,
    url: String,
) -> Result<bool, String> {
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    s.is_bookmarked(&url).map_err(|e| format!("is: {e}"))
}

#[tauri::command]
pub async fn bookmark_list(
    store: tauri::State<'_, crate::storage::SharedStore>,
) -> Result<Vec<crate::storage::Bookmark>, String> {
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    s.list_bookmarks().map_err(|e| format!("list: {e}"))
}

#[tauri::command]
pub async fn set_metasearch_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings =
        crate::search::SearchSettings::open(&data).map_err(|e| format!("open settings: {e}"))?;
    settings
        .set_metasearch(enabled)
        .map_err(|e| format!("set: {e}"))
}

/// Returns `true` when new tabs should spoof a mobile user-agent, so
/// the Settings radio stays in sync across restarts. Reads the
/// persisted value; `false` (desktop) on missing or unreadable.
#[tauri::command]
pub async fn get_mobile_ua(app: tauri::AppHandle) -> Result<bool, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings =
        crate::search::SearchSettings::open(&data).map_err(|e| format!("open settings: {e}"))?;
    settings.get_mobile_ua().map_err(|e| format!("get: {e}"))
}

/// Persist the desktop/mobile UA choice and rebuild every currently-
/// open tab so the flip applies globally. WKWebView / WebView2 can't
/// swap UA on a live webview, so every tab is closed and recreated
/// with the new UA (keeping URL, order, and the active selection).
/// Scroll position and form input are lost - unavoidable.
#[tauri::command]
pub async fn set_mobile_ua(app: tauri::AppHandle, mobile: bool) -> Result<(), String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings =
        crate::search::SearchSettings::open(&data).map_err(|e| format!("open settings: {e}"))?;
    settings
        .set_mobile_ua(mobile)
        .map_err(|e| format!("set: {e}"))?;

    // Rebuild existing tabs in place so the user doesn't need to
    // close + reopen each one manually for the change to take effect.
    crate::browser::rebuild_all_tabs(&app).await?;
    // Resize the whole window to a phone-shaped portrait (mobile) or
    // back to the desktop default - the webview is already narrow +
    // centered via active_tab_bounds, but the surrounding OS window
    // staying desktop-sized looked wrong around it.
    crate::browser::apply_window_size_for_mode(&app, mobile);
    Ok(())
}

/// UI-facing bundle of the Tor routing config plus what the currently-running
/// proxy was actually booted with. `applied_mode` is one of
/// `"direct"`, `"socks5:<addr>"`, `"built-in-tor"`.
#[derive(serde::Serialize)]
pub struct TorSettings {
    /// Whether the external SOCKS5 upstream is turned on.
    pub enabled: bool,
    /// Persisted SOCKS5 address (e.g. `127.0.0.1:9050`).
    pub proxy_addr: String,
    /// Whether the embedded Tor (arti) client should be used instead of
    /// an external SOCKS5 endpoint. Takes precedence over `enabled` at boot.
    pub built_in: bool,
    /// Whether the whole Tor stack was compiled in; grey out the built-in
    /// toggle in the UI when false.
    pub built_in_supported: bool,
    pub applied_mode: String,
}

#[tauri::command]
pub async fn get_tor_settings(
    app: tauri::AppHandle,
    state: State<'_>,
) -> Result<TorSettings, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings =
        crate::search::SearchSettings::open(&data).map_err(|e| format!("open settings: {e}"))?;
    let enabled = settings
        .get_tor_enabled()
        .map_err(|e| format!("get enabled: {e}"))?;
    let proxy_addr = settings
        .get_tor_proxy_addr()
        .map_err(|e| format!("get addr: {e}"))?;
    let built_in = settings
        .get_tor_builtin()
        .map_err(|e| format!("get built_in: {e}"))?;
    let applied_mode = {
        let s = state.lock().await;
        s.upstream_applied.clone()
    };
    Ok(TorSettings {
        enabled,
        proxy_addr,
        built_in,
        built_in_supported: cfg!(feature = "built-in-tor"),
        applied_mode,
    })
}

#[tauri::command]
pub async fn set_tor_settings(
    app: tauri::AppHandle,
    enabled: bool,
    proxy_addr: String,
    built_in: bool,
) -> Result<(), String> {
    if enabled {
        proxy_addr
            .parse::<std::net::SocketAddr>()
            .map_err(|e| format!("proxy_addr must be host:port (e.g. 127.0.0.1:9050): {e}"))?;
    }
    if built_in && !cfg!(feature = "built-in-tor") {
        return Err("this build was compiled without built-in Tor support".to_string());
    }
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings =
        crate::search::SearchSettings::open(&data).map_err(|e| format!("open settings: {e}"))?;
    settings
        .set_tor_enabled(enabled)
        .map_err(|e| format!("set enabled: {e}"))?;
    settings
        .set_tor_proxy_addr(&proxy_addr)
        .map_err(|e| format!("set addr: {e}"))?;
    settings
        .set_tor_builtin(built_in)
        .map_err(|e| format!("set built_in: {e}"))?;
    Ok(())
}

/// Path to the CA cert so the UI can point the user at it for manual trust install.
#[tauri::command]
pub async fn get_ca_cert_path(app: tauri::AppHandle) -> Result<String, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let cert = crate::ca::cert_path(&data_dir.join("ca"));
    Ok(cert.to_string_lossy().to_string())
}

/// UI-facing bundle of CA trust state.
#[derive(serde::Serialize)]
pub struct CaTrustStatus {
    /// Where the cert lives on disk.
    pub cert_path: String,
    /// Whether the OS trust store currently recognizes the CA.
    pub trusted: bool,
    /// Whether BlueFlame can attempt an auto-install on this platform.
    pub auto_install_supported: bool,
}

#[tauri::command]
pub async fn get_ca_trust_status(app: tauri::AppHandle) -> Result<CaTrustStatus, String> {
    let cert = resolve_cert_path(&app)?;
    Ok(CaTrustStatus {
        cert_path: cert.to_string_lossy().to_string(),
        trusted: crate::ca_trust::is_trusted(&cert),
        auto_install_supported: cfg!(target_os = "windows"),
    })
}

#[tauri::command]
pub async fn install_ca(app: tauri::AppHandle) -> Result<(), String> {
    let cert = resolve_cert_path(&app)?;
    crate::ca_trust::install(&cert).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reveal_ca(app: tauri::AppHandle) -> Result<String, String> {
    let cert = resolve_cert_path(&app)?;
    crate::ca_trust::reveal(&cert)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

fn resolve_cert_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    Ok(crate::ca::cert_path(&data_dir.join("ca")))
}

/// Result of a refresh - counts to show in the UI.
#[derive(serde::Serialize)]
pub struct RefreshResult {
    pub lists_ok: usize,
    pub lists_failed: usize,
    pub patterns_active: usize,
}

/// Dashboard one-liner: how long the proxy has been up, how many
/// patterns and lists are active right now, when the lists were last
/// refreshed (newest cache-file mtime).
#[derive(serde::Serialize)]
pub struct SystemSummary {
    pub uptime_secs: u64,
    pub patterns_active: usize,
    pub lists_total: usize,
    pub last_refresh_secs: Option<u64>,
}

#[tauri::command]
pub async fn get_system_summary(
    app: tauri::AppHandle,
    state: State<'_>,
) -> Result<SystemSummary, String> {
    let (started_at, patterns_active) = {
        let s = state.lock().await;
        let started = s.started_at;
        let count = s
            .filters
            .read()
            .map(|f| f.iter().map(|set| set.len()).sum::<usize>())
            .map_err(|e| format!("filters rwlock: {e}"))?;
        (started, count)
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let uptime_secs = started_at.map(|t| now.saturating_sub(t)).unwrap_or(0);

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let cache_dir = data_dir.join("filter-cache");
    let lists = crate::list_loader::default_lists();
    let lists_total = lists.len();
    let last_refresh_secs = lists
        .iter()
        .filter_map(|list| {
            let path = crate::list_loader::cache_path(&cache_dir, &list.url);
            std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        })
        .max();

    Ok(SystemSummary {
        uptime_secs,
        patterns_active,
        lists_total,
        last_refresh_secs,
    })
}

/// A filter list entry with its local cache status for display in Settings.
#[derive(serde::Serialize)]
pub struct FilterListEntry {
    pub name: String,
    pub url: String,
    /// `true` if we have a cached copy of this list on disk.
    pub cached: bool,
    /// Unix epoch seconds of the cache file's last-modified time; null if not cached.
    pub cached_at: Option<u64>,
}

#[tauri::command]
pub async fn get_filter_lists(app: tauri::AppHandle) -> Result<Vec<FilterListEntry>, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let cache_dir = data_dir.join("filter-cache");

    let lists = crate::list_loader::default_lists();
    Ok(lists
        .into_iter()
        .map(|list| {
            let path = crate::list_loader::cache_path(&cache_dir, &list.url);
            let cached_at = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            FilterListEntry {
                cached: cached_at.is_some(),
                cached_at,
                name: list.name,
                url: list.url,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn get_recent_blocks(
    state: State<'_>,
    limit: Option<usize>,
) -> Result<Vec<crate::proxy::BlockedEntry>, String> {
    let s = state.lock().await;
    let take = limit.unwrap_or(100).min(crate::proxy::BLOCK_LOG_CAPACITY);
    Ok(s.block_log.recent(take))
}

/// Count how many blocks in the recent log match a given host (exact host
/// or any subdomain of it). Cheap because the ring buffer is bounded.
#[tauri::command]
pub async fn get_trust(
    state: State<'_>,
    store: tauri::State<'_, crate::storage::SharedStore>,
    url: String,
) -> Result<crate::trust::TrustAssessment, String> {
    let host = url::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let (blocks, host_signals, reputation) = if host.is_empty() {
        (
            0usize,
            crate::security::HostSignals::default(),
            std::sync::Arc::new(crate::reputation::ReputationStore::with_bundled()),
        )
    } else {
        let s = state.lock().await;
        let needle = host.to_ascii_lowercase();
        let dot_suffix = format!(".{needle}");
        let blocks = s
            .block_log
            .recent(crate::proxy::BLOCK_LOG_CAPACITY)
            .into_iter()
            .filter(|entry| {
                url::Url::parse(&entry.url)
                    .ok()
                    .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
                    .is_some_and(|h| h == needle || h.ends_with(&dot_suffix))
            })
            .count();
        (
            blocks,
            s.security.signals_for(&needle),
            s.reputation.clone(),
        )
    };
    let assessment = crate::trust::evaluate(&url, blocks, &host_signals, &reputation);

    // Write a score sample so the panel can render a sparkline. 60s
    // throttle: fine-grained enough to catch score changes within a
    // session but coarse enough the table stays small over time.
    if !host.is_empty() {
        if let Ok(s) = store.lock() {
            let _ = s.record_trust_sample(&host.to_ascii_lowercase(), assessment.score, 60);
        }
    }

    Ok(assessment)
}

/// History of trust-score samples for `host`, oldest first. The
/// TrustPanel renders this as a sparkline next to the numeric score.
#[tauri::command]
pub async fn get_trust_history(
    store: tauri::State<'_, crate::storage::SharedStore>,
    host: String,
    limit: Option<usize>,
) -> Result<Vec<crate::storage::TrustSample>, String> {
    let needle = host.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let take = limit.unwrap_or(48).clamp(2, 500);
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
    s.trust_history(&needle, take)
        .map_err(|e| format!("trust history: {e}"))
}

#[tauri::command]
pub async fn get_blocks_for_host(state: State<'_>, host: String) -> Result<usize, String> {
    let s = state.lock().await;
    let needle = host.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Ok(0);
    }
    let dot_suffix = format!(".{needle}");
    let count = s
        .block_log
        .recent(crate::proxy::BLOCK_LOG_CAPACITY)
        .into_iter()
        .filter(|entry| {
            url::Url::parse(&entry.url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
                .is_some_and(|h| h == needle || h.ends_with(&dot_suffix))
        })
        .count();
    Ok(count)
}

#[tauri::command]
pub async fn clear_block_log(state: State<'_>) -> Result<(), String> {
    let s = state.lock().await;
    s.block_log.clear();
    Ok(())
}

#[tauri::command]
pub async fn reset_stats(state: State<'_>) -> Result<(), String> {
    let s = state.lock().await;
    s.stats
        .requests_total
        .store(0, std::sync::atomic::Ordering::Relaxed);
    s.stats
        .requests_blocked
        .store(0, std::sync::atomic::Ordering::Relaxed);
    s.stats
        .bytes_saved
        .store(0, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Re-download all default filter lists, recompile, and swap them in.
#[tauri::command]
pub async fn refresh_filter_lists(
    app: tauri::AppHandle,
    state: State<'_>,
) -> Result<RefreshResult, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let cache_dir = data_dir.join("filter-cache");
    std::fs::create_dir_all(&cache_dir).ok();

    let lists = crate::list_loader::default_lists();
    let mut fresh: Vec<String> = Vec::new();
    let mut failed: usize = 0;

    for list in &lists {
        match crate::list_loader::fetch(&list.url).await {
            Ok(body) => {
                let path = crate::list_loader::cache_path(&cache_dir, &list.url);
                let _ = crate::list_loader::write_cache(&path, &body);
                fresh.push(body);
            }
            Err(_) => failed += 1,
        }
    }

    let sets =
        crate::list_loader::compile_patterns(&crate::proxy::default_filter_patterns(), &fresh)
            .map_err(|e| format!("compile: {e}"))?;

    let patterns_active: usize = sets.iter().map(|s| s.len()).sum();
    {
        let s = state.lock().await;
        crate::proxy::swap_filters(&s, sets);
    }

    Ok(RefreshResult {
        lists_ok: fresh.len(),
        lists_failed: failed,
        patterns_active,
    })
}

/// Snapshot of each reputation feed subscription for the Settings
/// panel: name + URL + whether we have a cached copy + how old it is.
#[tauri::command]
pub async fn get_reputation_feeds(
    app: tauri::AppHandle,
) -> Result<Vec<crate::reputation::FeedStatus>, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let cache_dir = crate::reputation::cache_dir(&data_dir);
    Ok(crate::reputation::default_feeds()
        .iter()
        .map(|f| crate::reputation::status_for(f, &cache_dir))
        .collect())
}

/// Manual refresh trigger for reputation feeds. Fetches each feed,
/// re-caches, and unions the parsed hosts into the live store. Returns
/// a summary for the UI - same shape as the filter-list refresh so the
/// Settings panel renders it identically.
#[tauri::command]
pub async fn refresh_reputation_feeds(
    app: tauri::AppHandle,
    state: State<'_>,
) -> Result<RefreshResult, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let cache_dir = crate::reputation::cache_dir(&data_dir);
    std::fs::create_dir_all(&cache_dir).ok();

    let store = {
        let s = state.lock().await;
        s.reputation.clone()
    };

    let feeds = crate::reputation::default_feeds();
    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut added = 0usize;
    for feed in &feeds {
        match crate::reputation::fetch_and_cache(feed, &cache_dir).await {
            Ok(hosts) => {
                ok += 1;
                added += hosts.len();
                store.extend(hosts);
            }
            Err(e) => {
                tracing::warn!(error = ?e, name = %feed.name, "reputation refresh failed");
                failed += 1;
            }
        }
    }

    Ok(RefreshResult {
        lists_ok: ok,
        lists_failed: failed,
        // `patterns_active` is repurposed as the delta in listed hosts
        // for this refresh - the frontend renders the same summary line.
        patterns_active: added,
    })
}
