//! Wire protocol for the control pipe, and the `PageActions` trait that
//! abstracts the actual WebView2/DevTools calls away from the dispatcher.
//!
//! The pipe speaks newline-delimited JSON in both directions. The first
//! line from a client must be a [`Handshake`]; every line after that is a
//! [`Request`], answered with exactly one [`Response`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Handshake {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: u64,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}

/// Where a `click`, `type` or `scroll` call should act.
#[derive(Debug, Clone)]
pub enum ElementTarget {
    /// A `ref` returned by an earlier `read_page` or `find` call.
    Ref(u32),
    /// Viewport coordinates, `click` only.
    Point(f64, f64),
    /// No target: act on whatever currently has focus (`type`, `key`) or
    /// scroll the page itself (`scroll`).
    None,
}

/// The page-driving tools, implemented in-process against WebView2's own
/// DevTools Protocol support on Windows. Abstracted behind a trait so the
/// dispatcher (approval gate, tab scoping, logging) can be unit tested
/// against a fake instead of a real WebView2 instance.
#[async_trait]
pub trait PageActions: Send + Sync {
    async fn get_page_text(&self, tab_id: u64) -> Result<String, String>;
    async fn read_page(&self, tab_id: u64) -> Result<Value, String>;
    async fn find(&self, tab_id: u64, query: &str) -> Result<Value, String>;
    async fn click(&self, tab_id: u64, target: ElementTarget) -> Result<Value, String>;
    async fn type_text(
        &self,
        tab_id: u64,
        target: ElementTarget,
        text: &str,
    ) -> Result<Value, String>;
    async fn key(&self, tab_id: u64, key: &str) -> Result<Value, String>;
    async fn scroll(
        &self,
        tab_id: u64,
        target: ElementTarget,
        dx: f64,
        dy: f64,
    ) -> Result<Value, String>;
    async fn screenshot(&self, tab_id: u64) -> Result<Value, String>;
}
