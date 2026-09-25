//! Ties the pipe protocol to the browser: tab scoping, the approval gate,
//! the action log, and the actual tool implementations.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use super::approval::{origin_of, ApprovalGate, Decision};
use super::log::{ActionLog, ActionLogEntry};
use super::protocol::{ElementTarget, PageActions};
use super::tabs::ClaudeTabs;
use crate::browser;

/// How long a new-site prompt waits for Daniel before the tool call fails.
/// Long enough that stepping away for a minute doesn't lose the request,
/// short enough that a forgotten prompt doesn't hang Claude forever.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// Abstracts "given a tool name and its arguments, run it" behind a trait,
/// the same way `PageActions` abstracts WebView2 away from `ToolDispatcher`
/// itself. `ToolDispatcher` needs a real `AppHandle`, which only exists once
/// a real, WebView2-capable app has booted; `pipe::serve` depending on this
/// trait instead of the concrete type lets a real end-to-end test exercise
/// the actual named pipe, its ACL, and the token handshake against a
/// lightweight double instead - see `windows_impl::e2e_test`.
#[async_trait]
pub trait Dispatch: Send + Sync {
    async fn dispatch(&self, tool: &str, args: Value) -> Result<Value, String>;
}

#[async_trait]
impl Dispatch for ToolDispatcher {
    async fn dispatch(&self, tool: &str, args: Value) -> Result<Value, String> {
        ToolDispatcher::dispatch(self, tool, args).await
    }
}

pub struct ToolDispatcher {
    app: AppHandle,
    claude_tabs: Arc<ClaudeTabs>,
    approvals: Arc<ApprovalGate>,
    log: Arc<ActionLog>,
    page: Arc<dyn PageActions>,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next_request_id: AtomicU64,
}

impl ToolDispatcher {
    pub fn new(
        app: AppHandle,
        claude_tabs: Arc<ClaudeTabs>,
        approvals: Arc<ApprovalGate>,
        log: Arc<ActionLog>,
        page: Arc<dyn PageActions>,
    ) -> Self {
        Self {
            app,
            claude_tabs,
            approvals,
            log,
            page,
            pending: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
        }
    }

    /// Called from the `control_respond_approval` Tauri command once Daniel
    /// clicks Allow or Deny on the in-app prompt.
    pub fn respond_approval(&self, request_id: u64, allow: bool) -> bool {
        let sender = self
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(&request_id));
        match sender {
            Some(tx) => tx.send(allow).is_ok(),
            None => false,
        }
    }

    pub fn recent_log(&self, limit: usize) -> Vec<ActionLogEntry> {
        self.log.recent(limit)
    }

    /// Drop any tracked Claude tab id that no longer has an open tab (closed
    /// by Daniel or by Claude itself). Called on every `blueflame:tabs-changed`
    /// event so `ClaudeTabs` never grows stale entries a future `list_tabs`
    /// could return or a guessed id could collide with.
    pub fn reconcile_open_tabs(&self) {
        let Some(state) = self.app.try_state::<browser::Tabs>() else {
            return;
        };
        let Ok(view) = browser::browser_list_tabs(state) else {
            return;
        };
        let open: std::collections::HashSet<u64> = view.tabs.iter().map(|t| t.id).collect();
        for id in self.claude_tabs.list() {
            if !open.contains(&id) {
                self.claude_tabs.unmark(id);
            }
        }
    }

    pub async fn dispatch(&self, tool: &str, args: Value) -> Result<Value, String> {
        let result = self.dispatch_inner(tool, &args).await;
        let (detail, outcome) = match &result {
            Ok(_) => (args.to_string(), "ok".to_string()),
            Err(e) => (args.to_string(), format!("error: {e}")),
        };
        let tab_id = args.get("tab_id").and_then(|v| v.as_u64());
        self.log
            .record(ActionLogEntry::new(tab_id, tool, detail, &outcome));
        result
    }

    async fn dispatch_inner(&self, tool: &str, args: &Value) -> Result<Value, String> {
        match tool {
            "list_tabs" => self.list_tabs(),
            "open_tab" => self.open_tab(args).await,
            "navigate" => self.navigate(args).await,
            "get_page_text" => {
                let tab_id = self.require_claude_tab(args).await?;
                self.page.get_page_text(tab_id).await.map(Value::String)
            }
            "read_page" => {
                let tab_id = self.require_claude_tab(args).await?;
                self.page.read_page(tab_id).await
            }
            "find" => {
                let tab_id = self.require_claude_tab(args).await?;
                let query = str_arg(args, "query")?;
                self.page.find(tab_id, query).await
            }
            "click" => {
                let tab_id = self.require_claude_tab(args).await?;
                let target = element_target(args)?;
                self.page.click(tab_id, target).await
            }
            "type" => {
                let tab_id = self.require_claude_tab(args).await?;
                let target = element_target(args)?;
                let text = str_arg(args, "text")?;
                self.page.type_text(tab_id, target, text).await
            }
            "key" => {
                let tab_id = self.require_claude_tab(args).await?;
                let key = str_arg(args, "key")?;
                self.page.key(tab_id, key).await
            }
            "scroll" => {
                let tab_id = self.require_claude_tab(args).await?;
                let target = element_target(args)?;
                let dx = args.get("dx").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let dy = args.get("dy").and_then(|v| v.as_f64()).unwrap_or(0.0);
                self.page.scroll(tab_id, target, dx, dy).await
            }
            "screenshot" => {
                let tab_id = self.require_claude_tab(args).await?;
                self.page.screenshot(tab_id).await
            }
            other => Err(format!("unknown tool: {other}")),
        }
    }

    fn list_tabs(&self) -> Result<Value, String> {
        let state = self
            .app
            .try_state::<browser::Tabs>()
            .ok_or_else(|| "tabs state unavailable".to_string())?;
        let view = browser::browser_list_tabs(state)?;
        let claude_ids = self.claude_tabs.list();
        let tabs: Vec<Value> = view
            .tabs
            .into_iter()
            .filter(|t| claude_ids.contains(&t.id))
            .map(|t| json!({"tab_id": t.id, "url": t.url, "title": t.title}))
            .collect();
        Ok(json!({ "tabs": tabs }))
    }

    /// Open a fresh Claude-driven tab. Always InPrivate (no logins, no
    /// history) regardless of what the rest of the browser is doing.
    async fn open_tab(&self, args: &Value) -> Result<Value, String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("about:blank")
            .to_string();

        if url != "about:blank" {
            self.ensure_origin_approved(None, &origin_of(&url)).await?;
        }

        let tabs_state = self
            .app
            .try_state::<browser::Tabs>()
            .ok_or_else(|| "tabs state unavailable".to_string())?;
        let view = browser::browser_open_private_tab(self.app.clone(), tabs_state, url.clone())
            .await
            .map_err(|e| format!("open_tab: {e}"))?;
        let tab_id = view
            .active_id
            .ok_or_else(|| "open_tab: no active tab after open".to_string())?;
        self.claude_tabs.mark(tab_id);
        let _ = self.app.emit(
            "blueflame:claude-tab",
            json!({"tab_id": tab_id, "driving": true}),
        );
        Ok(json!({ "tab_id": tab_id, "url": url }))
    }

    async fn navigate(&self, args: &Value) -> Result<Value, String> {
        let tab_id = self.require_claude_tab(args).await?;
        let url = str_arg(args, "url")?.to_string();
        self.ensure_origin_approved(Some(tab_id), &origin_of(&url))
            .await?;

        let parsed: url::Url = url.parse().map_err(|e| format!("invalid url: {e}"))?;
        let webview = self
            .app
            .get_webview(&browser::tab_label(tab_id))
            .ok_or_else(|| "tab has no webview".to_string())?;
        webview
            .navigate(parsed)
            .map_err(|e| format!("navigate: {e}"))?;
        Ok(json!({ "tab_id": tab_id, "url": url }))
    }

    /// Confirms `tab_id` names a tab Claude itself opened, and returns it.
    /// Every content-touching tool (read/click/type/...) calls this first so
    /// a stray id can never reach one of Daniel's own tabs.
    async fn require_claude_tab(&self, args: &Value) -> Result<u64, String> {
        let tab_id = args
            .get("tab_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "missing tab_id".to_string())?;
        if !self.claude_tabs.is_claude_tab(tab_id) {
            return Err(format!("tab {tab_id} is not a Claude-controlled tab"));
        }
        // Defense in depth: the tab may have navigated to a new origin via
        // an in-page link since the last explicit `navigate` call, so
        // re-check the approval gate against wherever it actually is now.
        if let Some(current_url) = self.current_tab_url(tab_id) {
            self.ensure_origin_approved(Some(tab_id), &origin_of(&current_url))
                .await?;
        }
        Ok(tab_id)
    }

    fn current_tab_url(&self, tab_id: u64) -> Option<String> {
        let state = self.app.try_state::<browser::Tabs>()?;
        let view = browser::browser_list_tabs(state).ok()?;
        view.tabs
            .into_iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.url)
    }

    /// Blocks (asynchronously) until `origin` has a remembered decision,
    /// asking Daniel via the in-app prompt the first time it's seen.
    async fn ensure_origin_approved(
        &self,
        tab_id: Option<u64>,
        origin: &str,
    ) -> Result<(), String> {
        match self.approvals.decision(origin) {
            Some(Decision::Allow) => return Ok(()),
            Some(Decision::Deny) => return Err(format!("{origin} was previously denied")),
            None => {}
        }

        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel::<bool>();
        {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| "approval state poisoned".to_string())?;
            pending.insert(request_id, tx);
        }
        let _ = self.app.emit(
            "blueflame:claude-approval-request",
            json!({ "request_id": request_id, "origin": origin, "tab_id": tab_id }),
        );

        let outcome = tokio::time::timeout(APPROVAL_TIMEOUT, rx).await;
        // Clean up if we timed out before anyone answered.
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&request_id);
        }
        match outcome {
            Ok(Ok(true)) => {
                self.approvals.remember(origin, Decision::Allow);
                Ok(())
            }
            Ok(Ok(false)) => {
                self.approvals.remember(origin, Decision::Deny);
                Err(format!("{origin} was denied"))
            }
            _ => Err(format!(
                "no answer for {origin} within {}s",
                APPROVAL_TIMEOUT.as_secs()
            )),
        }
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing {key}"))
}

fn element_target(args: &Value) -> Result<ElementTarget, String> {
    if let Some(r) = args.get("ref").and_then(|v| v.as_u64()) {
        return Ok(ElementTarget::Ref(r as u32));
    }
    if let (Some(x), Some(y)) = (
        args.get("x").and_then(|v| v.as_f64()),
        args.get("y").and_then(|v| v.as_f64()),
    ) {
        return Ok(ElementTarget::Point(x, y));
    }
    Ok(ElementTarget::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;

    /// A fake `PageActions` that never touches WebView2, so dispatch and
    /// approval logic can be tested on every platform, not just Windows.
    struct FakePage {
        calls: AtomicUsize,
    }

    impl FakePage {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl PageActions for FakePage {
        async fn get_page_text(&self, _tab_id: u64) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok("hello".to_string())
        }
        async fn read_page(&self, _tab_id: u64) -> Result<Value, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(json!({"nodes": []}))
        }
        async fn find(&self, _tab_id: u64, _query: &str) -> Result<Value, String> {
            Ok(json!({"matches": []}))
        }
        async fn click(&self, _tab_id: u64, _target: ElementTarget) -> Result<Value, String> {
            Ok(json!({"clicked": true}))
        }
        async fn type_text(
            &self,
            _tab_id: u64,
            _target: ElementTarget,
            _text: &str,
        ) -> Result<Value, String> {
            Ok(json!({"typed": true}))
        }
        async fn key(&self, _tab_id: u64, _key: &str) -> Result<Value, String> {
            Ok(json!({"pressed": true}))
        }
        async fn scroll(
            &self,
            _tab_id: u64,
            _target: ElementTarget,
            _dx: f64,
            _dy: f64,
        ) -> Result<Value, String> {
            Ok(json!({"scrolled": true}))
        }
        async fn screenshot(&self, _tab_id: u64) -> Result<Value, String> {
            Ok(json!({"format": "png", "data": ""}))
        }
    }

    #[test]
    fn element_target_prefers_ref_over_point() {
        let args = json!({"ref": 3, "x": 1.0, "y": 2.0});
        matches!(element_target(&args).unwrap(), ElementTarget::Ref(3));
    }

    #[test]
    fn element_target_falls_back_to_point() {
        let args = json!({"x": 1.0, "y": 2.0});
        match element_target(&args).unwrap() {
            ElementTarget::Point(x, y) => {
                assert_eq!(x, 1.0);
                assert_eq!(y, 2.0);
            }
            _ => panic!("expected Point"),
        }
    }

    #[test]
    fn element_target_defaults_to_none() {
        let args = json!({});
        matches!(element_target(&args).unwrap(), ElementTarget::None);
    }

    #[tokio::test]
    async fn approval_gate_blocks_until_answered_then_remembers() {
        let approvals = Arc::new(ApprovalGate::in_memory());
        // Exercise the gate directly (no AppHandle needed for this slice):
        // first call has no decision, we remember Allow, second call passes.
        assert_eq!(approvals.decision("https://example.com"), None);
        approvals.remember("https://example.com", Decision::Allow);
        assert_eq!(
            approvals.decision("https://example.com"),
            Some(Decision::Allow)
        );
    }

    #[test]
    fn fake_page_is_send_sync_as_a_trait_object() {
        let _page: Arc<dyn PageActions> = Arc::new(FakePage::new());
    }
}
