//! Private tab-event channel for Windows: right-click / middle-click /
//! keyboard-relay events reach the shell over WebView2's own
//! `WebMessageReceived` event instead of Tauri IPC.
//!
//! Every `browse-*` tab webview's injected script (see
//! `CONTEXT_MENU_INIT_SCRIPT_TEMPLATE` in `browser.rs`) used to call
//! `submit_tab_event` through `window.__TAURI_INTERNALS__.invoke`. That
//! works for local content, but Tauri 2 refuses IPC from remote origins
//! unless the webview's capability grants a `remote` URL block - and
//! `tab-webviews.json` deliberately has none, because a `remote` block
//! there would hand every website on the internet the app's commands
//! (tabs, settings, downloads, the Claude control channel). So on real
//! web pages the injected script's `preventDefault()` still swallows the
//! native context menu, but the event describing what was clicked never
//! reaches Rust: right-click looks like it does nothing (see #11).
//!
//! The fix is a second, narrower channel that only exists on Windows:
//! the injected script posts the same event as a JSON string via
//! `window.chrome.webview.postMessage(JSON.stringify(...))`, tagged with a marker field
//! and the same per-launch token `submit_tab_event` already checks.
//! `attach_to_webview` below registers an additional
//! `ICoreWebView2::add_WebMessageReceived` handler on the tab's own
//! `ICoreWebView2` (reached through `Webview::with_webview`) that only
//! ever looks at messages carrying that marker and forwards validated
//! ones into the exact same `ContextMenuRequest` channel `submit_tab_event`
//! uses, via `context_menu::tab_event_to_request` /
//! `context_menu::route_tab_event`.
//!
//! It must be a string, not an object. wry registers its handler first
//! and reads every message with `TryGetWebMessageAsString(...)?`; for an
//! object that call fails, wry's handler returns the error, and WebView2
//! then skips every handler registered after it, ours included. Posting
//! objects is why right-click never reached Rust after #112.
//!
//! `add_WebMessageReceived` is additive, not exclusive: WebView2 fires
//! every registered handler for a `postMessage` call, and Tauri's own
//! Windows IPC bridge (wry) already has its own handler registered on
//! the same `ICoreWebView2` for its own `window.chrome.webview.postMessage`
//! traffic (its `__TAURI_INTERNALS__.invoke` calls go out exactly that
//! way on Windows). Adding ours does not remove or replace that handler,
//! so Tauri's IPC keeps working unchanged. Every message that is not one
//! of ours - including all of Tauri's own IPC frames - fails the marker
//! check in [`parse_tab_event_message`] and is silently ignored here,
//! and our own tagged messages have no shape Tauri's IPC parser expects,
//! so its handler discards them the same way it already discards any
//! other page traffic it doesn't recognize (it prints only the parse
//! error, "missing field `cmd`", to the page console, never the body).
//!
//! `submit_tab_event` (Tauri IPC) remains wired up as the fallback for
//! non-Windows platforms, unchanged.
//!
//! On a non-Windows build the parsing/validation below has no caller
//! outside its own tests (the registration code is Windows-only, same
//! restriction WebView2 itself has - see `control::mod`'s matching
//! attribute for the same reason). It is still unit tested on every CI
//! platform; only Windows clippy stays fully strict about it being used.
#![cfg_attr(not(windows), allow(dead_code))]

/// Marker field value the injected script tags its `postMessage` frames
/// with, so this handler can tell its own traffic apart from anything
/// else arriving over the same WebView2 event (notably Tauri's own IPC).
pub(crate) const MARKER: &str = "tab-event";

/// Upper bound on the raw message length this handler will even try to
/// parse. Generous for the real payloads (link/image URLs, a page URL,
/// up to 400 chars of selected text) while capping the cost of a
/// malicious page spamming huge `postMessage` frames at this channel.
pub(crate) const MAX_MESSAGE_BYTES: usize = 8 * 1024;

/// Why a `postMessage` frame was not treated as a tab event. Kept
/// distinct (rather than collapsing to a single error string) so unit
/// tests can assert on the specific reason, and so the handler can log
/// unexpected shapes differently from ordinary non-BlueFlame traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectReason {
    /// Longer than `MAX_MESSAGE_BYTES`; rejected before parsing.
    TooLarge,
    /// Not valid JSON, or missing/mistyped required fields.
    Malformed,
    /// Valid JSON but not tagged with `MARKER` - almost always someone
    /// else's traffic on the same WebView2 event, not an attack.
    WrongMarker,
    /// Tagged as ours but the token doesn't match this launch's token.
    BadToken,
}

/// One `postMessage` frame from the injected script, before the token
/// and marker are checked. `event` reuses `TabEvent`'s existing serde
/// shape so both delivery paths (this one and the Tauri IPC command)
/// accept exactly the same wire format.
#[derive(Debug, serde::Deserialize)]
struct Envelope {
    bf: String,
    token: String,
    event: crate::context_menu::TabEvent,
}

/// Validate a raw `postMessage` JSON string and, if it passes, return
/// the `TabEvent` it carries. Rejects anything oversized, malformed,
/// missing the marker, or carrying the wrong token - in that order, so
/// a too-large frame never reaches the JSON parser at all.
pub(crate) fn parse_tab_event_message(
    raw: &str,
    expected_token: &str,
) -> Result<crate::context_menu::TabEvent, RejectReason> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(RejectReason::TooLarge);
    }
    let envelope: Envelope = serde_json::from_str(raw).map_err(|_| RejectReason::Malformed)?;
    if envelope.bf != MARKER {
        return Err(RejectReason::WrongMarker);
    }
    if !crate::context_menu::token_matches(expected_token, &envelope.token) {
        return Err(RejectReason::BadToken);
    }
    Ok(envelope.event)
}

#[cfg(windows)]
mod windows_impl {
    //! The actual `ICoreWebView2::add_WebMessageReceived` registration.
    //! Only compiled on Windows, same restriction WebView2 itself has -
    //! see `control::windows_impl` for the sibling precedent using the
    //! same `webview2-com` bindings against the same tab webview.

    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2WebMessageReceivedEventArgs,
    };
    use webview2_com::{take_pwstr, WebMessageReceivedEventHandler};

    use crate::context_menu::SharedContextMenuTx;

    /// Register the handler on `webview`'s underlying `ICoreWebView2`.
    /// Dispatches onto the UI thread via `Webview::with_webview` (WebView2
    /// COM calls must run there); registration itself is fire-and-forget
    /// from the caller's point of view; failures are logged, not
    /// propagated, since the Tauri-IPC fallback still works if this
    /// somehow doesn't attach.
    pub(crate) fn attach_to_webview(
        webview: &tauri::Webview,
        tx_state: SharedContextMenuTx,
        token: String,
    ) {
        let result = webview.with_webview(move |platform| {
            if let Err(e) = attach_on_ui_thread(platform, tx_state, token) {
                tracing::warn!(error = %e, "failed to attach tab-event postMessage channel");
            }
        });
        if let Err(e) = result {
            tracing::warn!(error = %e, "with_webview dispatch failed for tab-event channel");
        }
    }

    /// Runs on the UI thread, inside the closure `with_webview` dispatches
    /// there. Mirrors `control::windows_impl::devtools::run_cdp_on_ui_thread`'s
    /// use of `platform.controller()` / `CoreWebView2()` to reach the same
    /// tab webview's COM object.
    fn attach_on_ui_thread(
        platform: tauri::webview::PlatformWebview,
        tx_state: SharedContextMenuTx,
        token: String,
    ) -> Result<(), String> {
        let controller = platform.controller();
        let core =
            unsafe { controller.CoreWebView2() }.map_err(|e| format!("CoreWebView2: {e}"))?;

        let handler = WebMessageReceivedEventHandler::create(Box::new(
            move |_sender: Option<ICoreWebView2>,
                  args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
                if let Some(args) = args {
                    handle_message(&args, &tx_state, &token);
                }
                Ok(())
            },
        ));

        let mut event_token: i64 = 0;
        unsafe { core.add_WebMessageReceived(&handler, &mut event_token) }
            .map_err(|e| format!("add_WebMessageReceived: {e}"))?;
        Ok(())
    }

    /// Pull the posted string out of the event args and hand it to the
    /// shared, platform-independent parser. Anything that isn't one of
    /// ours (wrong marker, bad token, malformed, oversized) is dropped
    /// silently - that includes Tauri's own IPC traffic, which arrives
    /// over this same WebView2 event and has no `bf` field at all.
    fn handle_message(
        args: &ICoreWebView2WebMessageReceivedEventArgs,
        tx_state: &SharedContextMenuTx,
        token: &str,
    ) {
        // Our script posts a JSON string. `WebMessageAsJson` would hand
        // back that string JSON-encoded a second time (a quoted literal),
        // which the parser rightly rejects, so read it as a string.
        let mut message = windows::core::PWSTR(std::ptr::null_mut());
        if unsafe { args.TryGetWebMessageAsString(&mut message) }.is_err() {
            return;
        }
        let raw = take_pwstr(message);
        let event = match super::parse_tab_event_message(&raw, token) {
            Ok(event) => event,
            Err(reason) => {
                tracing::debug!(?reason, "tab channel message dropped");
                return;
            }
        };
        if let Ok(request) = crate::context_menu::tab_event_to_request(event) {
            crate::context_menu::route_tab_event(tx_state, request);
        }
    }
}

#[cfg(windows)]
pub(crate) use windows_impl::attach_to_webview;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_menu::TabEvent;

    const TOKEN: &str = "abc123token";

    fn open_message(bf: &str, token: &str) -> String {
        format!(
            r#"{{"bf":"{bf}","token":"{token}","event":{{"kind":"open","page":"https://example.com","x":1,"y":2,"link":"","linkText":"","image":"","sel":""}}}}"#
        )
    }

    #[test]
    fn accepts_a_well_formed_open_event() {
        let raw = open_message(MARKER, TOKEN);
        let event = parse_tab_event_message(&raw, TOKEN).expect("should parse");
        assert!(matches!(event, TabEvent::Open { .. }));
    }

    #[test]
    fn rejects_wrong_marker() {
        let raw = open_message("something-else", TOKEN);
        assert_eq!(
            parse_tab_event_message(&raw, TOKEN).unwrap_err(),
            RejectReason::WrongMarker
        );
    }

    #[test]
    fn rejects_wrong_token() {
        let raw = open_message(MARKER, "not-the-token");
        assert_eq!(
            parse_tab_event_message(&raw, TOKEN).unwrap_err(),
            RejectReason::BadToken
        );
    }

    #[test]
    fn rejects_malformed_json() {
        let raw = "{ this is not json";
        assert_eq!(
            parse_tab_event_message(raw, TOKEN).unwrap_err(),
            RejectReason::Malformed
        );
    }

    #[test]
    fn rejects_json_missing_required_fields() {
        let raw = r#"{"bf":"tab-event"}"#;
        assert_eq!(
            parse_tab_event_message(raw, TOKEN).unwrap_err(),
            RejectReason::Malformed
        );
    }

    #[test]
    fn rejects_unknown_event_kind() {
        let raw = format!(
            r#"{{"bf":"{MARKER}","token":"{TOKEN}","event":{{"kind":"not-a-real-kind"}}}}"#
        );
        assert_eq!(
            parse_tab_event_message(&raw, TOKEN).unwrap_err(),
            RejectReason::Malformed
        );
    }

    #[test]
    fn rejects_oversized_messages_before_parsing() {
        let padding = "a".repeat(MAX_MESSAGE_BYTES + 1);
        let raw = format!(r#"{{"bf":"{MARKER}","token":"{TOKEN}","pad":"{padding}"}}"#);
        assert_eq!(
            parse_tab_event_message(&raw, TOKEN).unwrap_err(),
            RejectReason::TooLarge
        );
    }

    #[test]
    fn accepts_dismiss_kbd_and_middleclick_shapes() {
        let dismiss =
            format!(r#"{{"bf":"{MARKER}","token":"{TOKEN}","event":{{"kind":"dismiss"}}}}"#);
        assert!(matches!(
            parse_tab_event_message(&dismiss, TOKEN).unwrap(),
            TabEvent::Dismiss
        ));

        let kbd = format!(
            r#"{{"bf":"{MARKER}","token":"{TOKEN}","event":{{"kind":"kbd","key":"t","shift":true}}}}"#
        );
        assert!(matches!(
            parse_tab_event_message(&kbd, TOKEN).unwrap(),
            TabEvent::Kbd { key, shift } if key == "t" && shift
        ));

        let middleclick = format!(
            r#"{{"bf":"{MARKER}","token":"{TOKEN}","event":{{"kind":"middleclick","url":"https://example.com"}}}}"#
        );
        assert!(matches!(
            parse_tab_event_message(&middleclick, TOKEN).unwrap(),
            TabEvent::Middleclick { url } if url == "https://example.com"
        ));
    }

    #[test]
    fn end_to_end_routes_into_the_context_menu_channel() {
        let raw = open_message(MARKER, TOKEN);
        let event = parse_tab_event_message(&raw, TOKEN).expect("should parse");
        let request = crate::context_menu::tab_event_to_request(event).expect("should convert");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx_state: crate::context_menu::SharedContextMenuTx =
            std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
        crate::context_menu::route_tab_event(&tx_state, request);

        assert!(matches!(
            rx.try_recv().unwrap(),
            crate::context_menu::ContextMenuRequest::Open(_)
        ));
    }
}
