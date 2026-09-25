//! Claude control channel, phase 1.
//!
//! A local control server Claude Code talks to through a small MCP bridge
//! (see `mcp-bridge/` at the repo root), so Claude can drive BlueFlame the
//! same way it drives Chrome: open a page, read it, click, type.
//!
//! Everything platform-specific (the named pipe, its ACL, and the WebView2
//! DevTools calls that actually act on a page) lives under `windows/` and is
//! only compiled on Windows - the same restriction WebView2 itself has.
//! Everything else here (the token, the approval gate, tab scoping, the
//! action log, and tool dispatch) is plain Rust with no OS dependency, so it
//! is unit tested on every CI platform, not just this laptop.
//!
//! On a non-Windows build the only caller of most of this (`ToolDispatcher::
//! dispatch`, the pipe wire types, `SessionToken`) is `windows_impl::pipe`,
//! which doesn't exist there - so from `-D warnings` clippy's point of view
//! it is legitimately dead code on that platform, present purely so its own
//! unit tests still run in Linux CI. Windows clippy stays fully strict.
#![cfg_attr(not(windows), allow(dead_code))]

pub mod approval;
pub mod commands;
pub mod dispatch;
pub mod log;
pub mod protocol;
pub mod tabs;
pub mod token;

#[cfg(windows)]
mod windows_impl;

use std::sync::Arc;

use tauri::AppHandle;

use dispatch::ToolDispatcher;

/// Managed Tauri state wrapping the dispatcher, so the `control_*` Tauri
/// commands (approval responses, the log viewer) can reach it.
pub type SharedDispatcher = Arc<ToolDispatcher>;

/// Start the control channel: pick a session token, write it to the app
/// data directory, and spin up the named pipe server in the background.
/// No-op (with a log line) on platforms other than Windows.
#[cfg(windows)]
pub async fn start(app: &AppHandle) -> anyhow::Result<SharedDispatcher> {
    windows_impl::start(app).await
}

#[cfg(not(windows))]
pub async fn start(_app: &AppHandle) -> anyhow::Result<SharedDispatcher> {
    tracing::warn!("Claude control channel is only implemented on Windows in phase 1");
    Ok(Arc::new(ToolDispatcher::new(
        _app.clone(),
        Arc::new(tabs::ClaudeTabs::default()),
        Arc::new(approval::ApprovalGate::in_memory()),
        Arc::new(log::ActionLog::in_memory()),
        Arc::new(NullPageActions),
    )))
}

#[cfg(not(windows))]
struct NullPageActions;

#[cfg(not(windows))]
#[async_trait::async_trait]
impl protocol::PageActions for NullPageActions {
    async fn get_page_text(&self, _tab_id: u64) -> Result<String, String> {
        Err("Claude control channel is Windows-only in phase 1".into())
    }
    async fn read_page(&self, _tab_id: u64) -> Result<serde_json::Value, String> {
        Err("Claude control channel is Windows-only in phase 1".into())
    }
    async fn find(&self, _tab_id: u64, _query: &str) -> Result<serde_json::Value, String> {
        Err("Claude control channel is Windows-only in phase 1".into())
    }
    async fn click(
        &self,
        _tab_id: u64,
        _target: protocol::ElementTarget,
    ) -> Result<serde_json::Value, String> {
        Err("Claude control channel is Windows-only in phase 1".into())
    }
    async fn type_text(
        &self,
        _tab_id: u64,
        _target: protocol::ElementTarget,
        _text: &str,
    ) -> Result<serde_json::Value, String> {
        Err("Claude control channel is Windows-only in phase 1".into())
    }
    async fn key(&self, _tab_id: u64, _key: &str) -> Result<serde_json::Value, String> {
        Err("Claude control channel is Windows-only in phase 1".into())
    }
    async fn scroll(
        &self,
        _tab_id: u64,
        _target: protocol::ElementTarget,
        _dx: f64,
        _dy: f64,
    ) -> Result<serde_json::Value, String> {
        Err("Claude control channel is Windows-only in phase 1".into())
    }
    async fn screenshot(&self, _tab_id: u64) -> Result<serde_json::Value, String> {
        Err("Claude control channel is Windows-only in phase 1".into())
    }
}
