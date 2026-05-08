# Architecture

BlueFlame is a privacy-first browser shell built on Tauri 2. A native Rust
backend runs an embedded MITM filter proxy and owns all per-tab WebView
instances; a Preact + Vite frontend renders the chrome (tab strip, URL bar,
dashboard, settings, mobile UI). The system WebView (WebView2 on Windows,
WebKitGTK on Linux, WKWebView on macOS, Android WebView on mobile) is
configured at boot to route every request through `127.0.0.1:18080` so the
proxy can apply filters, gather signals, and proxy traffic upstream.

## Tech stack

- Backend: Rust (edition 2021), Tauri 2, Tokio
- MITM proxy: `hudsucker` 0.22, `hyper` 1, `hyper-rustls` 0.27, `rustls` 0.23
- Cert authority: `rcgen` 0.13 (self-signed root + per-host leaves)
- Storage: `rusqlite` 0.37 (bundled SQLite)
- Filters: `regex` 1 + `RegexSet`, custom easylist parser, parallel matching with `rayon`
- Tor: `arti-client` 0.41 (embedded, default feature) and SOCKS5 connector via `tokio-socks`
- HTTP fetcher: `reqwest` 0.12 (rustls-tls)
- HTML / body analysis: `scraper` 0.26, `x509-parser` 0.18
- Frontend: Preact 10 (via `@preact/preset-vite`), TypeScript, Vite 8, `lucide-react` icons
- Build: pnpm workspace, `tauri-cli` 2

## Component breakdown

Rust backend (`src-tauri/src/`):

- `lib.rs` / `main.rs`: entry point, rustls crypto provider install, WebView proxy
  configuration (Windows `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, Linux `http_proxy`,
  macOS per-webview `proxy_url`), session restore, filter-list and reputation-feed
  hydration, embedded-Tor bootstrap.
- `proxy.rs`: hudsucker MITM core, `ProxyState`, `ProxyStats`, `BlockLog` (500-entry
  ring buffer), filter dispatch, upstream selection (Direct / Socks5 / BuiltInTor).
- `ca.rs`, `ca_trust.rs`, `trust.rs`: persistent root CA at `<app_data>/ca/blueflame-ca.crt`,
  per-host trust scoring assembled from request, response, cert and body signals.
- `tls_verifier.rs`: custom rustls `ServerCertVerifier` that captures upstream cert
  metadata (issuer CN, validity window, sig alg, self-signed flag) without bypassing
  webpki chain verification.
- `filter_parser.rs`: easylist-line to regex translator. Skips `##`/`#@#`/`@@` rules
  and any line carrying `$options` (favors false negatives over breaking searches).
- `list_loader.rs`: fetches EasyPrivacy + EasyList, splits compiled rules across
  multiple `RegexSet`s to stay under the 128 MiB DFA guard.
- `reputation.rs`: URLHaus-format host feed, in-memory `HashSet<String>` plus disk cache.
- `body_analysis.rs`, `security.rs`: HTML scraping for cross-origin login forms,
  password-on-HTTP, outdated JS libs; per-host signal accumulator.
- `browser.rs`, `session.rs`, `new_tab.rs`: per-tab webview lifecycle, popup overlays
  (trust panel, menu, context menu), session save/restore via `<app_data>/session.json`.
- `context_menu.rs`: pre-warmed offscreen popup webview + IPC token so right-clicks
  show in roughly 0 ms after first launch.
- `downloads.rs`: Content-Disposition attachment handler, list view backend.
- `embedded_tor.rs`, `socks_connector.rs`: arti `TorClient` bootstrap + a
  `tower::Service<Uri>` connector; alternative SOCKS5 connector for external Tor.
- `storage.rs`: SQLite store at `<app_data>/personal.sqlite` (history, bookmarks,
  settings, trust history, suggestions). LIKE-based search with explicit escape.
- `metrics.rs`: self-process and descendant-webview RSS, CPU %, thread count.
- `favicons.rs`, `brand.rs`, `search.rs`, `metasearch.rs`, `import_export.rs`,
  `debug_log.rs`, `util.rs`: supporting surfaces.
- `commands.rs`: Tauri command handlers exposed to the frontend. The full list is
  registered in `lib.rs::run` via `tauri::generate_handler!`.

Frontend (`src/`):

- `main.tsx`, `App.tsx`, `App.css`: shell, view router, keyboard-shortcut dispatcher.
- `components/TitleBar.tsx`, `TabStrip.tsx`, `TabSwitcher.tsx`, `UrlBar.tsx`,
  `Sidebar.tsx`: browser chrome.
- `Bookmarks.tsx`, `BookmarksBar.tsx`, `Downloads.tsx`, `Settings.tsx`,
  `Dashboard.tsx`, `BlockLog.tsx`, `Metrics.tsx`, `Debug.tsx`, `FindBar.tsx`,
  `MobileChrome.tsx`, `PersonalIndex.tsx`: feature views.
- `ContextMenu.tsx`, `MenuPopup.tsx`, `CaTrustModal.tsx`, `TrustPopup.tsx`: overlays
  loaded into separate webviews so they can float above tab content.
- `useWebviewOverlay.ts`, `ascii.ts`: helpers for overlay layout and ASCII sparklines.

## Data flow

1. App boots. `lib.rs::run` installs the rustls ring provider, configures the
   WebView proxy env, and starts Tauri.
2. `start_proxy_at_boot` loads or creates the root CA, resolves the upstream
   (Direct, external SOCKS5 Tor, or embedded arti), and launches the hudsucker
   proxy on `127.0.0.1:18080`.
3. Background tasks hydrate filter lists (cached then fresh) and reputation feeds.
   `restore_session` reopens last-run tabs from `<app_data>/session.json`.
4. The user navigates. The system WebView sends every request to the proxy. The
   proxy matches against the active `RegexSet`, returns 204 on a match (incrementing
   `requests_blocked` and pushing to `BlockLog`), and otherwise dials upstream with
   the chosen connector.
5. Response bodies pass through `body_analysis` for HTML pages; signals land in
   `SecurityStore` keyed by page host. The trust scorer reads from there.
6. Frontend polls stats and trust data via Tauri commands every ~2 s; events
   (`blueflame:tab-shortcut`, etc.) push state changes the other way.

## Code organization

- `src-tauri/Cargo.toml`: dependency graph, dev/release profile tuning
  (`opt-level = "z"`, `lto = "fat"`, `panic = abort` for release; `opt-level = 1`
  for dev deps).
- `src-tauri/tauri.conf.json`: app config and bundle metadata.
- `src-tauri/gen/android/`: generated Gradle project for Android.
- `package.json`: pnpm scripts (`dev`, `build`, `preview`, `tauri`).
- `vite.config.ts`, `tsconfig.json`, `tsconfig.node.json`: frontend build.
- `.github/workflows/ci.yml`: build + test pipeline.
