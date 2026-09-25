//! A real, OS-level end-to-end exercise of the control channel: a real
//! named pipe (with the real ACL/SDDL code applied to it), a real
//! per-session token handshake (accepted and rejected), and the actual
//! `mcp-bridge` process speaking real MCP protocol through the official
//! SDK's `Client`/`StdioClientTransport` - the same transport Claude Code
//! itself uses to talk to a stdio MCP server.
//!
//! What this deliberately does NOT cover: `ToolDispatcher` itself (tab
//! scoping, the approval gate, logging - already unit tested directly in
//! `dispatch.rs`) and a real WebView2 tab. Both need a real, booted
//! `AppHandle` on the real `Wry` runtime: `open_tab` attaches a webview to
//! the app's `main` window (`browser::browser_open_private_tab`), and
//! `WindowsPageActions` then calls real `ICoreWebView2::
//! CallDevToolsProtocolMethod`. Booting a real `Wry` app even with zero
//! windows declared turned out not to be something a `cargo test` binary
//! can safely do on this laptop either (tried first; the resulting test
//! binary failed to start at all - `STATUS_ENTRYPOINT_NOT_FOUND` - which is
//! why `pipe::serve` now takes `Arc<dyn Dispatch>` instead of the concrete
//! `ToolDispatcher`, so this test can stand in a lightweight double
//! instead). Booting the *real* production app (real window, real proxy,
//! real app-data-derived CA/profile) from an unattended test would also
//! risk colliding with Daniel's own daily-driver BlueFlame instance - same
//! fixed pipe name, same proxy port, same profile - which is exactly what
//! the "never touch Daniel's real profile/CA" test rule guards against.
//!
//! So: `FakeDispatcher` below reuses the real `ClaudeTabs` (tab scoping)
//! and the real `PageActions` trait (`RecordingFakePage` standing in only
//! for WebView2), and implements just enough routing to dispatch
//! `get_page_text` / `read_page` / `click` / `type` - closing the "zero
//! end-to-end exercise" gap for the actual named pipe, its ACL, and the
//! token handshake, which is what PR #107's review comment was about.
//! `open_tab`/`navigate`/`list_tabs` and the approval-gate flow stay on
//! `ToolDispatcher`'s existing unit tests; a live WebView2 tab and a live
//! `claude` CLI session are Daniel's own manual smoke test per the README.

use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::ClientOptions;

use crate::control::dispatch::Dispatch;
use crate::control::protocol::{ElementTarget, PageActions};
use crate::control::tabs::ClaudeTabs;
use crate::control::token::SessionToken;

use super::acl::lock_down_file;
use super::pipe;

/// Stands in for `WindowsPageActions` (i.e. for WebView2) so the rest of
/// the pipe really is exercised end to end. Records every call it gets so
/// the test can assert the right arguments actually made it all the way
/// through the pipe and the dispatch routing.
struct RecordingFakePage {
    calls: Mutex<Vec<String>>,
    call_count: AtomicUsize,
}

impl RecordingFakePage {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            call_count: AtomicUsize::new(0),
        }
    }

    fn record(&self, call: impl Into<String>) {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.calls.lock().unwrap().push(call.into());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl PageActions for RecordingFakePage {
    async fn get_page_text(&self, tab_id: u64) -> Result<String, String> {
        self.record(format!("get_page_text({tab_id})"));
        Ok("hello from the real blueflame control pipe".to_string())
    }

    async fn read_page(&self, tab_id: u64) -> Result<Value, String> {
        self.record(format!("read_page({tab_id})"));
        Ok(json!({
            "nodes": [
                {"ref": "7", "role": "button", "name": "Submit", "value": null, "childIds": [], "ignored": false},
            ]
        }))
    }

    async fn find(&self, tab_id: u64, query: &str) -> Result<Value, String> {
        self.record(format!("find({tab_id}, {query})"));
        Ok(json!({ "matches": [] }))
    }

    async fn click(&self, tab_id: u64, target: ElementTarget) -> Result<Value, String> {
        let desc = match target {
            ElementTarget::Ref(r) => format!("click({tab_id}, ref={r})"),
            ElementTarget::Point(x, y) => format!("click({tab_id}, point=({x},{y}))"),
            ElementTarget::None => format!("click({tab_id}, none)"),
        };
        self.record(desc);
        Ok(json!({ "clicked": true }))
    }

    async fn type_text(
        &self,
        tab_id: u64,
        _target: ElementTarget,
        text: &str,
    ) -> Result<Value, String> {
        self.record(format!("type_text({tab_id}, {text:?})"));
        Ok(json!({ "typed": text.len() }))
    }

    async fn key(&self, tab_id: u64, key: &str) -> Result<Value, String> {
        self.record(format!("key({tab_id}, {key})"));
        Ok(json!({ "pressed": key }))
    }

    async fn scroll(
        &self,
        tab_id: u64,
        _target: ElementTarget,
        dx: f64,
        dy: f64,
    ) -> Result<Value, String> {
        self.record(format!("scroll({tab_id}, {dx}, {dy})"));
        Ok(json!({ "scrolled": true }))
    }

    async fn screenshot(&self, tab_id: u64) -> Result<Value, String> {
        self.record(format!("screenshot({tab_id})"));
        Ok(json!({ "format": "png", "data": "" }))
    }
}

/// A minimal `Dispatch` for this test only: real tab scoping (`ClaudeTabs`,
/// the same struct production code uses) plus routing for the four tools
/// the PR's "Done when" bar names (open/read/click/type - `open_tab` itself
/// is simulated by pre-marking the tab, since a real one needs a real
/// webview). Everything else `ToolDispatcher` adds on top of this routing -
/// the approval gate, the action log, `open_tab`/`navigate`/`list_tabs` -
/// is exercised by its own unit tests in `dispatch.rs`, not duplicated
/// here.
struct FakeDispatcher {
    claude_tabs: Arc<ClaudeTabs>,
    page: Arc<RecordingFakePage>,
}

impl FakeDispatcher {
    fn require_tab(&self, args: &Value) -> Result<u64, String> {
        let tab_id = args
            .get("tab_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "missing tab_id".to_string())?;
        if !self.claude_tabs.is_claude_tab(tab_id) {
            return Err(format!("tab {tab_id} is not a Claude-controlled tab"));
        }
        Ok(tab_id)
    }
}

fn element_target(args: &Value) -> ElementTarget {
    if let Some(r) = args.get("ref").and_then(|v| v.as_u64()) {
        return ElementTarget::Ref(r as u32);
    }
    ElementTarget::None
}

#[async_trait]
impl Dispatch for FakeDispatcher {
    async fn dispatch(&self, tool: &str, args: Value) -> Result<Value, String> {
        match tool {
            "get_page_text" => {
                let tab_id = self.require_tab(&args)?;
                self.page.get_page_text(tab_id).await.map(Value::String)
            }
            "read_page" => {
                let tab_id = self.require_tab(&args)?;
                self.page.read_page(tab_id).await
            }
            "click" => {
                let tab_id = self.require_tab(&args)?;
                self.page.click(tab_id, element_target(&args)).await
            }
            "type" => {
                let tab_id = self.require_tab(&args)?;
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing text".to_string())?;
                self.page
                    .type_text(tab_id, element_target(&args), text)
                    .await
            }
            other => Err(format!("unsupported by this test double: {other}")),
        }
    }
}

/// Connect to a named pipe, retrying while it doesn't exist yet
/// (`pipe::serve`'s first `CreateNamedPipeW` is a separate task, so a
/// brand-new pipe name needs a short settle window) or is momentarily
/// "busy" (Windows named pipes only ever have exactly as many pending
/// instances as `pipe::serve`'s loop has created so far; connecting
/// consumes one, and the loop only creates the next after the current one
/// is accepted). A plain retry-until-connected here, rather than a
/// separate throwaway "is it up yet" probe, matters because a probe
/// connection would itself consume a pipe instance without the server ever
/// getting a handshake line from it.
async fn connect_retrying(pipe_name: &str) -> tokio::net::windows::named_pipe::NamedPipeClient {
    for _ in 0..200 {
        match ClientOptions::new().open(pipe_name) {
            Ok(client) => return client,
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    panic!("control pipe {pipe_name} never came up");
}

#[ignore = "spawns a real named pipe server + the real Node mcp-bridge; run explicitly on the Windows laptop with mcp-bridge's deps installed (pnpm install at the repo root)"]
#[tokio::test]
async fn real_pipe_end_to_end_through_the_bridge() {
    // A real, disposable app-data root - never Daniel's real
    // `%APPDATA%\com.w1ck3ds0d4.blueflame`. Deleted (via tempdir's Drop) at
    // the end of the test.
    let app_data = tempfile::tempdir().expect("tempdir");
    let bridge_app_data = app_data.path().join("bridge-app-data");
    let token_dir = bridge_app_data
        .join("com.w1ck3ds0d4.blueflame")
        .join("control");
    std::fs::create_dir_all(&token_dir).expect("create token dir");
    let token = SessionToken::generate();
    let token_path = token_dir.join("token");
    std::fs::write(&token_path, token.as_str()).expect("write token");
    // Same real ACL code path production uses - proves it doesn't just
    // compile, it succeeds against a real file this test owns and deletes.
    lock_down_file(&token_path).expect("lock down test token file");

    // A pipe name unique to this test process, so it can never collide
    // with - or be mistaken by a client for - a real BlueFlame instance's
    // `blueflame-control` pipe.
    let pipe_name = format!(r"\\.\pipe\blueflame-control-test-{}", std::process::id());

    let claude_tabs = Arc::new(ClaudeTabs::default());
    const TAB_ID: u64 = 42;
    claude_tabs.mark(TAB_ID); // simulates open_tab already having run

    let page = Arc::new(RecordingFakePage::new());
    let dispatcher: Arc<dyn Dispatch> = Arc::new(FakeDispatcher {
        claude_tabs,
        page: page.clone(),
    });

    let serve_pipe_name = pipe_name.clone();
    let serve_token = token.clone();
    let server_task = tokio::spawn(async move {
        let _ = pipe::serve(&serve_pipe_name, serve_token, dispatcher).await;
    });

    // 1. The pipe really does refuse a caller without the right token, over
    //    the real OS transport (not the mocked line-reader in pipe.rs's own
    //    unit tests). Also doubles as "wait for the pipe to come up".
    {
        let client = connect_retrying(&pipe_name).await;
        let (r, mut w) = tokio::io::split(client);
        let mut reader = BufReader::new(r);
        w.write_all(br#"{"token":"not-the-real-token"}"#)
            .await
            .unwrap();
        w.write_all(b"\n").await.unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(
            line.contains(r#""ok":false"#),
            "expected the pipe to reject a wrong token, got: {line}"
        );
    }

    // 2. The real mcp-bridge process (the exact file Claude Code starts,
    //    per README.md), driven over stdio by the official MCP client SDK
    //    (the same transport/protocol Claude Code uses), reads this token
    //    file and talks to this pipe.
    let bridge_dir = format!("{}/../mcp-bridge", env!("CARGO_MANIFEST_DIR"));
    let driver_path = format!("{bridge_dir}/test/e2e-driver.mjs");
    assert!(
        std::path::Path::new(&driver_path).exists(),
        "missing {driver_path}"
    );
    assert!(
        std::path::Path::new(&format!("{bridge_dir}/node_modules")).exists(),
        "mcp-bridge/node_modules is missing - run `pnpm install` at the repo root first"
    );

    let calls = json!([
        {"name": "get_page_text", "args": {"tab_id": TAB_ID}},
        {"name": "read_page", "args": {"tab_id": TAB_ID}},
        {"name": "click", "args": {"tab_id": TAB_ID, "ref": 7}},
        {"name": "type", "args": {"tab_id": TAB_ID, "ref": 7, "text": "hello claude"}},
    ]);

    let output = tokio::process::Command::new("node")
        .arg(&driver_path)
        .arg(calls.to_string())
        .env("BLUEFLAME_APP_DATA", &bridge_app_data)
        .env("BLUEFLAME_PIPE_NAME", &pipe_name)
        .current_dir(&bridge_dir)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("spawn the real mcp-bridge driver");

    server_task.abort();

    if !output.status.success() {
        panic!(
            "e2e driver failed (status {:?}):\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("driver did not print JSON ({e}): {stdout}"));

    let tools = parsed["tools"].as_array().expect("tools array");
    for expected in [
        "list_tabs",
        "open_tab",
        "navigate",
        "get_page_text",
        "read_page",
        "find",
        "click",
        "type",
        "key",
        "scroll",
        "screenshot",
    ] {
        assert!(
            tools.iter().any(|t| t == expected),
            "bridge did not advertise tool {expected}"
        );
    }

    let results = parsed["results"].as_array().expect("results array");
    for r in results {
        assert!(
            r["ok"].as_bool().unwrap_or(false),
            "tool call {} failed: {:?}",
            r["name"],
            r["error"]
        );
    }

    let get_text = &results[0]["content"][0]["text"];
    assert_eq!(
        get_text.as_str().unwrap(),
        "hello from the real blueflame control pipe"
    );

    let read_page_text = results[1]["content"][0]["text"].as_str().unwrap();
    assert!(read_page_text.contains("Submit"));

    // The dispatcher really did route each call, over the real pipe, all
    // the way to the fake WebView2 leaf, with the right arguments.
    let recorded = page.calls();
    assert!(recorded.iter().any(|c| c == "get_page_text(42)"));
    assert!(recorded.iter().any(|c| c == "read_page(42)"));
    assert!(recorded.iter().any(|c| c == "click(42, ref=7)"));
    assert!(recorded
        .iter()
        .any(|c| c == "type_text(42, \"hello claude\")"));
}
