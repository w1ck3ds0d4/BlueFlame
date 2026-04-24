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
