//! `PageActions` implemented in-process against WebView2's own DevTools
//! Protocol support: `ICoreWebView2::CallDevToolsProtocolMethod`, reached
//! through Tauri's `Webview::with_webview`. No remote-debugging port is ever
//! opened - see `SECURITY.md` and the ROADMAP item this ships.
//!
//! CDP calls only ever run against the tab's own webview, dispatched onto
//! the UI thread by `with_webview`. `webview2_com`'s
//! `wait_for_async_operation` blocks that dispatched closure (pumping
//! Windows messages, which is how the completion callback itself gets
//! delivered) until WebView2 answers, so the call looks synchronous from
//! inside the closure; the async wrapper below just waits on a channel for
//! that closure to finish, with a timeout so a wedged page can't hang the
//! MCP bridge forever.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;

use crate::browser;
use crate::control::protocol::{ElementTarget, PageActions};

const CDP_TIMEOUT: Duration = Duration::from_secs(10);

/// Per-tab cache of the last `Accessibility.getFullAXTree` result, so
/// `find` and `click`/`type`/`scroll` by `ref` don't need a fresh DOM
/// round-trip: `ref` is the CDP AX `nodeId`, mapped here to the
/// `backendDOMNodeId` that `DOM.resolveNode` needs.
pub struct WindowsPageActions {
    app: AppHandle,
    ax_cache: Mutex<HashMap<u64, HashMap<u32, i64>>>,
}

impl WindowsPageActions {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            ax_cache: Mutex::new(HashMap::new()),
        }
    }

    async fn call(&self, tab_id: u64, method: &str, params: Value) -> Result<Value, String> {
        call_cdp(&self.app, tab_id, method, params).await
    }

    fn cache_nodes(&self, tab_id: u64, nodes: &[Value]) {
        let mut map = HashMap::new();
        for node in nodes {
            let Some(node_id) = node
                .get("nodeId")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            if let Some(backend) = node.get("backendDOMNodeId").and_then(|v| v.as_i64()) {
                map.insert(node_id, backend);
            }
        }
        if let Ok(mut cache) = self.ax_cache.lock() {
            cache.insert(tab_id, map);
        }
    }

    fn backend_id_for_ref(&self, tab_id: u64, node_ref: u32) -> Option<i64> {
        self.ax_cache
            .lock()
            .ok()?
            .get(&tab_id)?
            .get(&node_ref)
            .copied()
    }

    async fn resolve_object_id(&self, tab_id: u64, node_ref: u32) -> Result<String, String> {
        if self.backend_id_for_ref(tab_id, node_ref).is_none() {
            // Cache miss: refresh from a fresh read_page before giving up.
            self.read_page(tab_id).await?;
        }
        let backend_id = self
            .backend_id_for_ref(tab_id, node_ref)
            .ok_or_else(|| format!("unknown ref {node_ref}: call read_page again"))?;
        let resolved = self
            .call(
                tab_id,
                "DOM.resolveNode",
                json!({ "backendNodeId": backend_id }),
            )
            .await?;
        resolved
            .pointer("/object/objectId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "DOM.resolveNode returned no objectId".to_string())
    }

    async fn resolve_target(
        &self,
        tab_id: u64,
        target: &ElementTarget,
    ) -> Result<Option<String>, String> {
        match target {
            ElementTarget::Ref(r) => Ok(Some(self.resolve_object_id(tab_id, *r).await?)),
            ElementTarget::Point(_, _) | ElementTarget::None => Ok(None),
        }
    }
}

#[async_trait]
impl PageActions for WindowsPageActions {
    async fn get_page_text(&self, tab_id: u64) -> Result<String, String> {
        let result = self
            .call(
                tab_id,
                "Runtime.evaluate",
                json!({
                    "expression": "document.body ? document.body.innerText : ''",
                    "returnByValue": true,
                }),
            )
            .await?;
        Ok(result
            .pointer("/result/value")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string())
    }

    async fn read_page(&self, tab_id: u64) -> Result<Value, String> {
        let _ = self.call(tab_id, "Accessibility.enable", json!({})).await;
        let tree = self
            .call(tab_id, "Accessibility.getFullAXTree", json!({}))
            .await?;
        let nodes = tree
            .get("nodes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        self.cache_nodes(tab_id, &nodes);

        let simplified: Vec<Value> = nodes
            .iter()
            .map(|node| {
                json!({
                    "ref": node.get("nodeId").and_then(|v| v.as_str()).unwrap_or(""),
                    "role": node.pointer("/role/value"),
                    "name": node.pointer("/name/value"),
                    "value": node.pointer("/value/value"),
                    "childIds": node.get("childIds").cloned().unwrap_or(Value::Array(vec![])),
                    "ignored": node.get("ignored").cloned().unwrap_or(Value::Bool(false)),
                })
            })
            .collect();
        Ok(json!({ "nodes": simplified }))
    }

    async fn find(&self, tab_id: u64, query: &str) -> Result<Value, String> {
        let page = self.read_page(tab_id).await?;
        let needle = query.to_lowercase();
        let nodes = page
            .get("nodes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let matches: Vec<Value> = nodes
            .into_iter()
            .filter(|node| {
                let role = node.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let value = node.get("value").and_then(|v| v.as_str()).unwrap_or("");
                role.to_lowercase().contains(&needle)
                    || name.to_lowercase().contains(&needle)
                    || value.to_lowercase().contains(&needle)
            })
            .collect();
        Ok(json!({ "matches": matches }))
    }

    async fn click(&self, tab_id: u64, target: ElementTarget) -> Result<Value, String> {
        match &target {
            ElementTarget::Point(x, y) => {
                self.call(
                    tab_id,
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
                )
                .await?;
                self.call(
                    tab_id,
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
                )
                .await?;
                Ok(json!({ "clicked": "point" }))
            }
            ElementTarget::Ref(_) => {
                let object_id = self
                    .resolve_target(tab_id, &target)
                    .await?
                    .ok_or_else(|| "click: no element resolved".to_string())?;
                self.call_function_on(tab_id, &object_id, "function(){this.scrollIntoView({block:'center',inline:'center'});this.click();return true;}")
                    .await?;
                Ok(json!({ "clicked": "ref" }))
            }
            ElementTarget::None => Err("click needs a ref or x/y".to_string()),
        }
    }

    async fn type_text(
        &self,
        tab_id: u64,
        target: ElementTarget,
        text: &str,
    ) -> Result<Value, String> {
        if let Some(object_id) = self.resolve_target(tab_id, &target).await? {
            self.call_function_on(tab_id, &object_id, "function(){this.focus();}")
                .await?;
        }
        self.call(tab_id, "Input.insertText", json!({ "text": text }))
            .await?;
        Ok(json!({ "typed": text.len() }))
    }

    async fn key(&self, tab_id: u64, key: &str) -> Result<Value, String> {
        let (code, windows_vk, key_str) = map_key(key)?;
        for event_type in ["keyDown", "keyUp"] {
            self.call(
                tab_id,
                "Input.dispatchKeyEvent",
                json!({
                    "type": event_type,
                    "key": key_str,
                    "code": code,
                    "windowsVirtualKeyCode": windows_vk,
                    "nativeVirtualKeyCode": windows_vk,
                }),
            )
            .await?;
        }
        Ok(json!({ "pressed": key }))
    }

    async fn scroll(
        &self,
        tab_id: u64,
        target: ElementTarget,
        dx: f64,
        dy: f64,
    ) -> Result<Value, String> {
        if let Some(object_id) = self.resolve_target(tab_id, &target).await? {
            self.call_function_on(
                tab_id,
                &object_id,
                "function(){this.scrollIntoView({block:'center',inline:'center'});}",
            )
            .await?;
            return Ok(json!({ "scrolled": "ref" }));
        }
        self.call(
            tab_id,
            "Runtime.evaluate",
            json!({ "expression": format!("window.scrollBy({dx}, {dy})") }),
        )
        .await?;
        Ok(json!({ "scrolled": "page" }))
    }

    async fn screenshot(&self, tab_id: u64) -> Result<Value, String> {
        let result = self
            .call(tab_id, "Page.captureScreenshot", json!({ "format": "png" }))
            .await?;
        let data = result
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Page.captureScreenshot returned no data".to_string())?;
        Ok(json!({ "format": "png", "data": data }))
    }
}

impl WindowsPageActions {
    async fn call_function_on(
        &self,
        tab_id: u64,
        object_id: &str,
        function_declaration: &str,
    ) -> Result<Value, String> {
        self.call(
            tab_id,
            "Runtime.callFunctionOn",
            json!({ "objectId": object_id, "functionDeclaration": function_declaration }),
        )
        .await
    }
}

/// Common special keys a form-filling or navigation flow needs. Anything
/// not in this table is rejected rather than guessed at.
fn map_key(name: &str) -> Result<(&'static str, i32, &'static str), String> {
    Ok(match name {
        "Enter" => ("Enter", 0x0D, "Enter"),
        "Tab" => ("Tab", 0x09, "Tab"),
        "Escape" => ("Escape", 0x1B, "Escape"),
        "Backspace" => ("Backspace", 0x08, "Backspace"),
        "Delete" => ("Delete", 0x2E, "Delete"),
        "ArrowUp" => ("ArrowUp", 0x26, "ArrowUp"),
        "ArrowDown" => ("ArrowDown", 0x28, "ArrowDown"),
        "ArrowLeft" => ("ArrowLeft", 0x25, "ArrowLeft"),
        "ArrowRight" => ("ArrowRight", 0x27, "ArrowRight"),
        "Home" => ("Home", 0x24, "Home"),
        "End" => ("End", 0x23, "End"),
        "PageUp" => ("PageUp", 0x21, "PageUp"),
        "PageDown" => ("PageDown", 0x22, "PageDown"),
        "Space" => ("Space", 0x20, " "),
        other => return Err(format!("unsupported key: {other}")),
    })
}

/// Run one CDP method against `tab_id`'s webview and return the parsed JSON
/// result. Dispatches onto the UI thread via `with_webview` and waits
/// (off the UI thread) for that dispatched closure to report back, bounded
/// by `CDP_TIMEOUT` so a hung page can't hang the bridge indefinitely - the
/// UI thread itself may still be blocked inside WebView2's message pump
/// until the call resolves or the tab is closed, a known limitation of the
/// synchronous DevTools completion API documented in the PR.
async fn call_cdp(
    app: &AppHandle,
    tab_id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let webview = app
        .get_webview(&browser::tab_label(tab_id))
        .ok_or_else(|| format!("tab {tab_id} has no webview"))?;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let method_owned = method.to_string();
    let params_json = params.to_string();

    webview
        .with_webview(move |platform| {
            let result = run_cdp_on_ui_thread(platform, &method_owned, &params_json);
            let _ = tx.send(result);
        })
        .map_err(|e| format!("with_webview: {e}"))?;

    let raw = match tokio::time::timeout(CDP_TIMEOUT, rx).await {
        Ok(Ok(inner)) => inner?,
        Ok(Err(_)) => return Err("devtools call channel closed".to_string()),
        Err(_) => return Err(format!("devtools call to {method} timed out")),
    };
    serde_json::from_str(&raw).map_err(|e| format!("devtools call returned invalid JSON: {e}"))
}

/// Everything below runs synchronously on the UI thread, inside the closure
/// `with_webview` dispatches there.
fn run_cdp_on_ui_thread(
    platform: tauri::webview::PlatformWebview,
    method: &str,
    params_json: &str,
) -> Result<String, String> {
    let controller = platform.controller();
    let core = unsafe { controller.CoreWebView2() }.map_err(|e| format!("CoreWebView2: {e}"))?;

    // Build explicit PCWSTR values rather than letting `&str` convert via
    // `Param<PCWSTR>`: this crate and `webview2-com` each depend on
    // `windows`/`windows-core` in their own right, and the dependency tree
    // carries more than one semver line of that crate (arti and sysinfo
    // both pull a newer one for unrelated reasons), which makes the
    // blanket `&str` conversion ambiguous. Building the wide string
    // ourselves and passing a `PCWSTR` sidesteps that entirely.
    let method_wide = windows::core::HSTRING::from(method);
    let params_wide = windows::core::HSTRING::from(params_json);
    let method_pcwstr = windows::core::PCWSTR(method_wide.as_ptr());
    let params_pcwstr = windows::core::PCWSTR(params_wide.as_ptr());

    let holder: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let holder_for_completion = holder.clone();

    let outcome = CallDevToolsProtocolMethodCompletedHandler::wait_for_async_operation(
        Box::new(move |completed_handler| unsafe {
            core.CallDevToolsProtocolMethod(method_pcwstr, params_pcwstr, &completed_handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |error_code, json_result| {
            *holder_for_completion.borrow_mut() = Some(json_result);
            error_code
        }),
    );

    outcome.map_err(|e| format!("{method}: {e:?}"))?;
    let result = holder
        .borrow_mut()
        .take()
        .ok_or_else(|| format!("{method}: no result returned"));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_key_covers_enter_and_rejects_unknown() {
        assert!(map_key("Enter").is_ok());
        assert!(map_key("NotAKey").is_err());
    }
}
