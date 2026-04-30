# BlueFlame

Privacy-first browser shell. On desktop, an embedded MITM filter proxy strips trackers and analytics at the network layer. On Android, the same filter rules run via WebView's native `shouldInterceptRequest` hook - no proxy, no CA trust needed. iOS will follow via `WKContentRuleList` once a Mac is available for the build.

---

## Features

### Privacy + filtering

- **Embedded MITM proxy** - intercepts all WebView traffic locally (HTTP and HTTPS) via the `hudsucker` crate
- **Self-signed root CA** - generated on first run and persisted to the user data dir; key never leaves disk
- **Built-in blocklist** - covers the worst offenders out of the box (doubleclick, GA, GTM, hotjar, mixpanel, segment, amplitude, fb pixels)
- **Filter list support** - EasyPrivacy + EasyList subscribed by default, easylist-compatible rules compiled into SQLite, live-reload without restart
- **Body analysis + reputation** - request/response analysis hooks plus a URL reputation pass for fingerprinting + tracking heuristics
- **Embedded Tor (via arti)** - SOCKS connector wired through to the proxy so any tab can route over Tor without an external client
- **Private tabs** - sessions isolated from the main store

### Browser shell

- **Tabs + tab switcher** - middle-click a link to open in a new BlueFlame tab; relayed `Ctrl+T/W/L/F/R/D/Tab/1-9` keyboard shortcuts from inside tab webviews
- **Bookmarks** - slash-path folder tree, full-page browser, mobile kebab entry, Netscape `bookmarks.html` import, settings JSON backup/restore
- **Downloads** - saves Content-Disposition attachments with a list view UI
- **Themed right-click context menu** - browser-style actions (open, copy link, view source, inspect) across desktop and mobile long-press; suppresses the native menu and routes F12 to the in-app Debug view
- **Find-in-page** - dedicated `FindBar` component, `Ctrl+F` from any tab
- **Search bar + metasearch** - configurable engines including Ahmia for `.onion` indexing
- **Resource monitor panel** - ASCII sparklines for CPU / memory / network alongside the privacy dashboard
- **New-tab page + branded UI** - gradient logo, lucide icon set across navigation
- **Mobile (Android) chrome** - WebView's native `shouldInterceptRequest` does the filtering, no proxy or CA trust step needed; dedicated mobile UI components

### Storage + observability

- **SQLite storage** - history, bookmarks, settings, filter lists, downloads stored locally
- **Privacy dashboard** - live counter of requests total, requests blocked, bytes saved
- **Block log** - per-request view of what the proxy stripped, in real time
- **No telemetry** - BlueFlame itself phones home to nothing; the only network traffic is what you browse

---

## Setup

### Prerequisites

- [Rust stable toolchain](https://rustup.rs/) (1.90+)
- [Node.js](https://nodejs.org/) 20+
- [pnpm](https://pnpm.io/installation)
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS (WebView2 on Windows, `libwebkit2gtk-4.1` on Linux, Xcode tools on macOS)

For Android builds:

- Android Studio (for the SDK + AVD manager)
- JDK 17+
- Android SDK platform-tools, platforms, build-tools 34+, NDK 27+
- Android Rust targets: `rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android`
- Env vars: `JAVA_HOME`, `ANDROID_HOME`, `NDK_HOME`

### Clone and install

```bash
git clone https://github.com/w1ck3ds0d4/BlueFlame.git
cd BlueFlame
pnpm install
```

### Run in dev mode

```bash
pnpm tauri dev
```

First launch generates `blueflame-ca.crt` and `blueflame-ca.key` in the app data directory. You will be prompted to trust the cert so the proxy can intercept HTTPS.

### Build a release

```bash
pnpm tauri build
```

Output goes to `src-tauri/target/release/`.

### Android dev

```bash
pnpm tauri android dev    # runs on a connected device or emulator
pnpm tauri android build  # produces APK/AAB in src-tauri/gen/android/app/build/outputs/
```

The Android build uses `WebView.shouldInterceptRequest` directly - no proxy, no CA trust step. Filters match the desktop built-in set.

---

## Usage

### Start the proxy

The proxy auto-starts when the app launches and listens on `localhost:18080`. The WebView is configured at startup to route through it. Use the **Enable filters / Disable filters** button in the dashboard to toggle blocking without restarting the proxy.

### Trust the CA

On first run the app creates `blueflame-ca.crt`. Your OS needs to recognize it or HTTPS sites will show cert warnings. BlueFlame shows a first-run modal offering to handle this for you.

| OS | Auto-install | Manual fallback |
|---|---|---|
| Windows | Yes - the modal runs `certutil -user -addstore Root ...` which pops a confirmation dialog. No admin required. | Double-click the `.crt` and install into "Trusted Root Certification Authorities". |
| macOS | Not yet | `sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain blueflame-ca.crt` |
| Linux | Not yet | `sudo cp blueflame-ca.crt /usr/local/share/ca-certificates/ && sudo update-ca-certificates` |

**Security note:** installing any root CA is a serious action - anyone who gets the CA private key could impersonate HTTPS sites on that machine. BlueFlame keeps the key locally in the app data dir alongside the cert. If you stop using BlueFlame, uninstall the CA from your trust store.

### View stats

The dashboard updates every 2 seconds with totals for requests, blocks, and estimated bytes saved.

### Filter lists

BlueFlame ships with EasyPrivacy and EasyList subscribed by default. On first launch it:

1. Compiles the built-in minimal rule set into the active filter immediately so the proxy blocks the worst offenders even offline.
2. Loads any cached filter list bodies from `<app_data>/filter-cache/` and hot-swaps the merged set in.
3. Fetches fresh copies of each list in the background, writes them to the cache, and swaps again.

Failures (bad HTTP, malformed lines, invalid regex) are logged and skipped - a single bad list line will not break filtering.

Invoke the `refresh_filter_lists` Tauri command (or, later, click Refresh in Settings) to re-download on demand. The command returns `{ lists_ok, lists_failed, patterns_active }`.

---

## Project Structure

```
BlueFlame/
  src/                                  React + Vite frontend
    main.tsx                            Entry
    App.tsx                             Shell, tab strip, chrome
    App.css                             Dark theme styles
    ascii.ts                            ASCII sparkline + glyph helpers
    useWebviewOverlay.ts                Hook for popup webviews
    components/
      TitleBar.tsx, TabStrip.tsx,       Browser chrome
        TabSwitcher.tsx, UrlBar.tsx,
        Sidebar.tsx
      Bookmarks.tsx,                    Bookmarks tree + bar
        BookmarksBar.tsx
      Downloads.tsx                     Download list view
      ContextMenu.tsx, MenuPopup.tsx    Right-click + dropdown menus
      FindBar.tsx                       Ctrl+F find-in-page
      Dashboard.tsx, BlockLog.tsx,      Privacy dashboard + per-request log
        Metrics.tsx                     Resource monitor sparklines
      CaTrustModal.tsx, TrustPopup.tsx  CA install prompts
      Settings.tsx, Debug.tsx           Settings page + F12 view
      MobileChrome.tsx                  Mobile-shaped UI for Android
      PersonalIndex.tsx                 New-tab dashboard
  src-tauri/                            Rust Tauri backend
    src/
      lib.rs, main.rs                   App entry + binary wrapper
      commands.rs                       Tauri commands exposed to frontend
      proxy.rs                          hudsucker MITM proxy, stats, filtering
      ca.rs, ca_trust.rs, trust.rs      Root CA generation, persistence, OS trust install
      tls_verifier.rs                   Custom TLS cert verification
      storage.rs                        SQLite (history, bookmarks, settings, filter lists, downloads)
      filter_parser.rs, list_loader.rs  EasyList rule parser + remote list fetcher
      body_analysis.rs, reputation.rs   Request/response heuristics + URL reputation pass
      browser.rs, session.rs, new_tab.rs Tab session management + new-tab page
      context_menu.rs                   Right-click action backend
      downloads.rs                      Content-Disposition attachment handler
      embedded_tor.rs, socks_connector.rs Arti-backed Tor SOCKS proxy
      favicons.rs                       Favicon fetch + cache
      brand.rs                          Logo + branding assets
      search.rs, metasearch.rs          Search bar + engine routing
      metrics.rs                        Resource monitor backend
      import_export.rs                  Settings JSON + Netscape bookmarks HTML
      security.rs                       Security utility surface
      debug_log.rs, util.rs             Logging + helpers
    Cargo.toml                          Rust deps (tauri, hudsucker, rcgen, rusqlite,
                                          arti-client, tor-rtcompat)
    tauri.conf.json                     Tauri app config
    gen/android/                        Android Studio + Gradle project
    build.rs                            Build-time code gen
  .github/
    workflows/ci.yml                    Build + test
    dependabot.yml
  .vscode/                              Launch + tasks + recommended extensions
  package.json                          Frontend deps + Tauri CLI scripts
  vite.config.ts                        Vite dev server + build config
  LICENSE                               AGPL v3
  COMMERCIAL.md                         Commercial license terms
  README.md
```

---

## License

This project is dual-licensed:

- **[AGPL v3](LICENSE)** - free for open-source use. Derivatives and SaaS deployments must release their source under AGPL.
- **[Commercial license](COMMERCIAL.md)** - for proprietary / closed-source use or hosted services that do not want to comply with AGPL source-disclosure requirements. Contact for terms.
