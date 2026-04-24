//! System-metrics snapshot: what the BlueFlame process tree is
//! consuming right now. Surfaced to the "metrics" panel via a single
//! `get_system_metrics` Tauri command that the frontend polls on a
//! ~2s cadence to build sparklines.
//!
//! "Self" here means the BlueFlame process + its child webview
//! processes (WebView2 on Windows, WebKitGTK on Linux, WKWebView on
//! macOS). We sum RSS and CPU across the process tree so the single
//! number on the panel reflects the real memory/CPU the app is
//! costing you, not just the Rust host process.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::proxy::ProxyState;

/// One point-in-time resource snapshot of the BlueFlame process tree
/// plus proxy / tab / filter counters. Fields are organized by
/// subsystem so the frontend can render them into distinct cards.
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    /// Unix seconds when we took the snapshot. Used by the frontend
    /// to throttle duplicate samples if the poll fires twice in the
    /// same second.
    pub ts: u64,

    // ── process self ──────────────────────────────────────────────
    pub pid: u32,
    /// Seconds since BlueFlame started (parent PID only; child webview
    /// processes are usually younger).
    pub uptime_secs: u64,
    /// Resident-set size in bytes, summed across the BlueFlame parent
    /// and every descendant webview/helper process.
    pub rss_bytes: u64,
    /// CPU percentage summed across the tree. Can exceed 100% on
    /// multi-core systems (each core contributes up to 100%).
    pub cpu_percent: f32,
    /// Total thread count across the tree. `None` on platforms where
    /// sysinfo doesn't expose thread counts.
    pub thread_count: Option<u64>,
    /// How many processes made up the tree this sample.
    pub process_count: u32,

    // ── proxy ─────────────────────────────────────────────────────
    pub proxy_requests_total: u64,
    pub proxy_requests_blocked: u64,
    pub proxy_bytes_saved: u64,

    // ── browser surfaces ──────────────────────────────────────────
    pub tab_count: u32,
    pub private_tab_count: u32,
}

/// Long-lived `System` wrapped in a `Mutex` so both `get_system_metrics`
/// invocations share the same sampling state. Required for CPU %:
/// sysinfo computes `cpu_usage()` as the delta between the last two
/// refreshes, so the first call after construction always returns 0.0.
/// Keeping it alive across calls lets subsequent polls see real deltas.
pub struct MetricsCollector {
    sys: Mutex<System>,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self {
            sys: Mutex::new(System::new()),
        }
    }
}

impl MetricsCollector {
    /// Refresh the process table and return a snapshot. Holds the
    /// mutex across the refresh + walk; both are fast (~low ms), so
    /// callers serializing on this won't noticeably contend.
    pub fn sample(&self, tabs: &crate::browser::Tabs, proxy: &ProxyState) -> MetricsSnapshot {
        let self_pid = std::process::id();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut sys = self.sys.lock().expect("metrics system mutex poisoned");
        // Refresh just process-level info. Skip disk/user because we
        // don't display them and they're the slowest to collect.
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory().with_cpu(),
        );

        let tree_pids = descendants_including(&sys, Pid::from_u32(self_pid));
        let mut rss_bytes: u64 = 0;
        let mut cpu_percent: f32 = 0.0;

        for pid in &tree_pids {
            if let Some(proc_) = sys.process(*pid) {
                rss_bytes = rss_bytes.saturating_add(proc_.memory());
                cpu_percent += proc_.cpu_usage();
            }
        }
        // sysinfo 0.33 only exposes per-process thread counts on Linux
        // (via `tasks()`). On Windows and macOS the field is `None`;
        // rather than lie with 0, surface it as `None` so the UI can
        // render "n/a" for unsupported platforms.
        let thread_count: Option<u64> = None;

        let uptime_secs = sys
            .process(Pid::from_u32(self_pid))
            .map(|p| p.run_time())
            .unwrap_or(0);

        let stats = proxy.stats.snapshot();
        let (tab_count, private_tab_count) = tabs.lock().map(|g| g.tab_counts()).unwrap_or((0, 0));

        MetricsSnapshot {
            ts,
            pid: self_pid,
            uptime_secs,
            rss_bytes,
            cpu_percent,
            thread_count,
            process_count: tree_pids.len() as u32,
            proxy_requests_total: stats.requests_total,
            proxy_requests_blocked: stats.requests_blocked,
            proxy_bytes_saved: stats.bytes_saved,
            tab_count,
            private_tab_count,
        }
    }
}

/// Collect `root` plus every process that transitively has `root` as
/// an ancestor. Walks `parent()` links rather than following children
/// because sysinfo exposes the former directly.
fn descendants_including(sys: &System, root: Pid) -> Vec<Pid> {
    let mut out = Vec::new();
    if sys.process(root).is_some() {
        out.push(root);
    }
    for (pid, proc_) in sys.processes() {
        if *pid == root {
            continue;
        }
        let mut cursor = proc_.parent();
        while let Some(p) = cursor {
            if p == root {
                out.push(*pid);
                break;
            }
            cursor = sys.process(p).and_then(|parent_proc| parent_proc.parent());
        }
    }
    out
}

pub type SharedMetrics = std::sync::Arc<MetricsCollector>;

#[tauri::command]
pub async fn get_system_metrics(
    metrics: tauri::State<'_, SharedMetrics>,
    tabs: tauri::State<'_, crate::browser::Tabs>,
    state: tauri::State<'_, std::sync::Arc<tokio::sync::Mutex<ProxyState>>>,
) -> Result<MetricsSnapshot, String> {
    // Grab the ProxyState guard first (async tokio lock), then run the
    // sysinfo refresh synchronously. Both are fast enough (~low ms each)
    // that holding the async lock across the refresh is fine.
    let guard = state.lock().await;
    Ok(metrics.sample(&tabs, &guard))
}
