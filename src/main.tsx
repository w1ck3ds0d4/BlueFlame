import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import { ContextMenu } from "./components/ContextMenu";
import { MenuPopup } from "./components/MenuPopup";
import { TrustPopup } from "./components/TrustPopup";

// Pipe frontend runtime failures into the same log the Debug view reads
// from the backend. Fire-and-forget so a broken IPC doesn't cascade.
function logToBackend(level: string, target: string, message: string) {
  invoke("log_from_frontend", { level, target, message }).catch(() => {
    /* ignore - the log call itself failed, nothing we can do */
  });
}

window.addEventListener("error", (e) => {
  const where = e.filename ? ` @ ${e.filename}:${e.lineno}:${e.colno}` : "";
  logToBackend("error", "frontend:onerror", `${e.message}${where}`);
});

// Suppress the native webview context menu on every BlueFlame webview
// (shell + menu/trust/context popups). Without this, right-clicking on
// a button or the sidebar shows the web-page menu (Reload / Back /
// View source / Inspect), which leaks the Chromium-browser feel and
// fights our theme. Tab webviews have their own themed context menu
// handled server-side; this covers everything else.
// Exception: inputs + textareas + contentEditable keep the native menu
// so the user still has Cut/Copy/Paste/Spell-check in the URL bar.
window.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement | null;
  if (!t) return;
  const tag = t.tagName;
  const editable = t.isContentEditable === true;
  if (tag === "INPUT" || tag === "TEXTAREA" || editable) return;
  e.preventDefault();
});

window.addEventListener("unhandledrejection", (e) => {
  const reason = e.reason instanceof Error ? e.reason.message : String(e.reason);
  logToBackend("error", "frontend:promise", `unhandled rejection: ${reason}`);
});

// Mirror console.warn / console.error so third-party noise and our own
// warnings both end up in the debug feed without losing the original
// devtools output.
const origWarn = console.warn.bind(console);
const origError = console.error.bind(console);
console.warn = (...args: unknown[]) => {
  origWarn(...args);
  logToBackend("warn", "frontend:console", args.map(fmtArg).join(" "));
};
console.error = (...args: unknown[]) => {
  origError(...args);
  logToBackend("error", "frontend:console", args.map(fmtArg).join(" "));
};

function fmtArg(a: unknown): string {
  if (a instanceof Error) return a.message;
  if (typeof a === "string") return a;
  try {
    return JSON.stringify(a);
  } catch {
    return String(a);
  }
}

// Fork on entry: child webviews loaded with `?panel=<kind>` mount a
// dedicated popup component and skip the full App shell (which would
// kick off proxy polling, tab restore, etc.).
const panelParam = new URLSearchParams(window.location.search).get("panel");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {panelParam === "trust" ? (
      <TrustPopup />
    ) : panelParam === "menu" ? (
      <MenuPopup />
    ) : panelParam === "context" ? (
      <ContextMenu />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
